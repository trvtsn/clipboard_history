pub mod cache;
pub mod commands;
pub mod crypto;

use crate::cache::{
    DecryptedCache, DecryptedCacheState, DecryptedPreviewCache, DecryptedPreviewCacheState,
    HistoryCacheState, wipe_history_cache, write_history_cache, write_settings_cache,
};
use crate::commands::*;
use crate::crypto::{
    EncryptedRecord, HistoryEntry, PlaintextPayload, Vault, VaultState, content_hash,
    parse_recipient,
};
pub use clipboard_history::AppError;
use clipboard_history::constants::{
    HISTORY_KEY, HISTORY_STORE_FILE, SETTINGS_KEY, SETTINGS_STORE_FILE,
};
use clipboard_history::{
    AppSettings, CopiedObject, CopiedObjectPreview, ObjectContent, ObjectFormat,
};
use clipboard_rs::{
    Clipboard, ClipboardContext, ClipboardHandler, ClipboardWatcher, ClipboardWatcherContext,
    ContentFormat, common::RustImage,
};
use parking_lot::{Mutex, RwLock};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::App;
use tauri::{
    AppHandle, Emitter, Manager, WindowEvent, Wry,
    menu::{CheckMenuItem, Menu, MenuItem},
    tray::TrayIconBuilder,
};
use tauri_plugin_store::StoreExt;
use zeroize::{Zeroize, Zeroizing};

/// `HistoryWriteLock`` just ensures multiple threads can't write to the history.json
/// file at the same time, ensuring integrity of data.
pub type HistoryWriteLock = Mutex<()>;
// RwLock as we're doing more reads than writes with this.
/// `SettingsState` stores the settings in memory for faster lookup rather than
/// relying on fetching from disk all the time.
pub type SettingsState = RwLock<AppSettings>;

/// Stores the hash of the most recently copied object's content. Used to skip
/// consecutive duplicate copies.
pub type LastHashState = Mutex<Option<String>>;

pub struct CapturePaused(pub AtomicBool);

/// Holds a handle to the tray's "Pause capture" checkbox. Keeps the value in sync 
/// when toggled from the frontend UI.
pub struct PauseMenuItem(pub CheckMenuItem<Wry>);

struct ClipboardManager {
    ctx: ClipboardContext,
    app: AppHandle,
}

impl ClipboardManager {
    pub fn new(app: AppHandle) -> Self {
        let ctx = ClipboardContext::new().expect("failed to init clipboard context");
        Self { ctx, app }
    }

    fn detect_content_type(&self) -> Option<ContentFormat> {
        [
            ContentFormat::Files,
            ContentFormat::Image,
            ContentFormat::Text,
            ContentFormat::Html,
            ContentFormat::Rtf,
        ]
        .into_iter()
        .find(|format| self.ctx.has(format.clone()))
    }

    fn read_clipboard(&self) -> Option<CopiedObject> {
        let ctx = &self.ctx;
        let (content, content_format, thumbnail, formatted_content) =
            match self.detect_content_type()? {
                ContentFormat::Files => {
                    let files = ctx.get_files().ok()?;
                    (ObjectContent::Files(files), ObjectFormat::Files, None, None)
                }
                ContentFormat::Image => {
                    let img = ctx.get_image().ok()?;
                    let png = img.to_png().ok()?;
                    let thumbnail = img
                        .thumbnail(120, 80)
                        .and_then(|thumbnail| thumbnail.to_png())
                        .ok()
                        .map(|buf| buf.get_bytes().to_vec());
                    (
                        ObjectContent::Image(png.get_bytes().to_vec()),
                        ObjectFormat::Image,
                        thumbnail,
                        None,
                    )
                }
                ContentFormat::Text => {
                    let text = ctx.get_text().ok()?;
                    let html = ctx
                        .has(ContentFormat::Html)
                        .then(|| ctx.get_html().ok().filter(|html| !html.is_empty()))
                        .flatten();
                    let rtf = || {
                        ctx.has(ContentFormat::Rtf)
                            .then(|| ctx.get_rich_text().ok().filter(|rtf| !rtf.is_empty()))
                            .flatten()
                    };
                    let (format, formatted) = match html {
                        Some(html) => (ObjectFormat::Html, Some(html)),
                        None => match rtf() {
                            Some(rtf) => (ObjectFormat::Rtf, Some(rtf)),
                            None => (ObjectFormat::Text, None),
                        },
                    };
                    (ObjectContent::Text(text), format, None, formatted)
                }
                ContentFormat::Html => {
                    let html = ctx.get_html().ok()?;
                    (ObjectContent::Html(html), ObjectFormat::Html, None, None)
                }
                ContentFormat::Rtf => {
                    let rtf = ctx.get_rich_text().ok()?;
                    (ObjectContent::Rtf(rtf), ObjectFormat::Rtf, None, None)
                }
                ContentFormat::Other(name) => {
                    let buf = ctx.get_buffer(&name).ok()?;
                    (
                        ObjectContent::Other(name.clone(), buf),
                        ObjectFormat::Other(name),
                        None,
                        None,
                    )
                }
            };

        Some(CopiedObject {
            id: 0, // Placeholder, overwritten in `append_to_history`
            content,
            content_format,
            date: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|duration| duration.as_millis() as u64)
                .unwrap_or(0),
            thumbnail,
            formatted_content,
        })
    }
}

impl ClipboardHandler for ClipboardManager {
    fn on_clipboard_change(&mut self) {
        let paused = self
            .app
            .try_state::<CapturePaused>()
            .is_some_and(|paused| paused.0.load(Ordering::Relaxed));
        if paused {
            return;
        }

        let Some(mut object) = self.read_clipboard() else {
            return;
        };

        if !append_to_history(&self.app, &mut object).is_ok_and(|id| id.is_some()) {
            return;
        }
        if let Err(e) = prune_expired(&self.app) {
            eprintln!("failed to prune history: {e}");
        }

        let unlocked_or_unencrypted = self
            .app
            .try_state::<VaultState>()
            .map(|vault| {
                let vault = vault.read();
                vault.recipient.is_none() || vault.identity.is_some()
            })
            .unwrap_or(true);

        if unlocked_or_unencrypted {
            let mut preview = CopiedObjectPreview::from_full(&object);
            if let Err(e) = self.app.emit("new_copied_object", &preview) {
                eprintln!("failed to emit: {e}");
            }
            preview.zeroize();
        }
        // } else if let Err(e) = self.app.emit("new_copied_object_locked", &object.id) {
        //     eprintln!("failed to emit: {e}");
        // }

        object.zeroize();
    }
}

/// Gets a snapshot of the saved clipboard history entries from either the cached memory or from history.json.
pub fn history_snapshot(app: &AppHandle) -> Result<Arc<Vec<HistoryEntry>>, AppError> {
    let cache = app.state::<HistoryCacheState>();
    if let Some(entries) = cache.read().clone() {
        return Ok(entries);
    }
    let entries = Arc::new(read_history(app)?);
    *cache.write() = Some(entries.clone());
    Ok(entries)
}

fn append_to_history(app: &AppHandle, object: &mut CopiedObject) -> Result<Option<u32>, AppError> {
    let history_write_lock = app.state::<HistoryWriteLock>();
    let _h = history_write_lock.lock();

    let recipient = app
        .try_state::<VaultState>()
        .and_then(|v| v.read().recipient.clone());

    let mut history = Arc::unwrap_or_clone(history_snapshot(app)?);

    let new_hash = content_hash(&object.content);
    let last_hash_state = app.state::<LastHashState>();
    let is_dup = {
        let last = last_hash_state.lock();
        if let Some(h) = last.as_deref() {
            h == new_hash
        } else if let Some(HistoryEntry::Plain(o)) = history.last() {
            content_hash(&o.content) == new_hash
        } else {
            false
        }
    };
    if is_dup {
        return Ok(None);
    }

    let id = match history.iter().map(|entry| entry.id()).max() {
        Some(max) => max.checked_add(1).ok_or_else(|| {
            AppError::Custom("History ID space exhausted. Clear some entries.".into())
        })?,
        None => 1,
    };
    object.id = id;

    let entry = if let Some(recipient) = recipient {
        let payload = Zeroizing::new(PlaintextPayload {
            id: Some(object.id),
            date: Some(object.date),
            content: object.content.clone(),
            content_format: object.content_format.clone(),
            thumbnail: object.thumbnail.clone(),
            formatted_content: object.formatted_content.clone(),
        });
        let ciphertext = payload.encrypt_to_recipient(&recipient)?;
        HistoryEntry::Encrypted(EncryptedRecord {
            id: object.id,
            date: object.date,
            ciphertext,
        })
    } else {
        HistoryEntry::Plain(object.clone())
    };

    history.push(entry);
    write_history(app, &history)?;
    *last_hash_state.lock() = Some(new_hash);
    Ok(Some(id))
}

fn load_vault_recipient(app: &AppHandle) -> Result<(), AppError> {
    let settings_state = app.state::<SettingsState>();
    let settings = settings_state.read();

    let Some(ref cfg) = settings.encryption else {
        return Ok(());
    };

    let recipient = parse_recipient(&cfg.recipient)?;
    let state = app.state::<VaultState>();
    let mut guard = state.write();
    guard.recipient = Some(recipient);
    Ok(())
}

fn prune_expired(app: &AppHandle) -> Result<(), AppError> {
    let settings_state = app.state::<SettingsState>();
    let (retention_amount, retention_unit) = {
        let settings = settings_state.read();
        (settings.retention_amount, settings.retention_unit)
    };

    if retention_amount == 0 {
        return Ok(());
    }

    let identity = {
        let vault = app.state::<VaultState>();
        let guard = vault.read();
        if guard.recipient.is_some() && guard.identity.is_none() {
            return Ok(());
        }
        guard.identity.clone()
    };

    let history_write_lock = app.state::<HistoryWriteLock>();
    let _h = history_write_lock.lock();

    let mut history = Arc::unwrap_or_clone(history_snapshot(app)?);

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0);
    let window = retention_amount.saturating_mul(retention_unit.ms_per_unit());
    let cutoff = now.saturating_sub(window);

    let before = history.len();
    let mut pruned_ids = Vec::new();
    history.retain(|item| {
        if item.date() >= cutoff {
            return true;
        }
        let expired = match item {
            HistoryEntry::Plain(_) => true,
            HistoryEntry::Encrypted(encrypted_record) => match identity.as_ref() {
                Some(id) => encrypted_record
                    .decrypt_with_identity(id)
                    .map(|mut payload| {
                        let expired = payload.authoritative_date(encrypted_record) < cutoff;
                        payload.zeroize();
                        expired
                    })
                    .unwrap_or(false),
                None => false,
            },
        };
        if expired {
            pruned_ids.push(item.id());
        }
        !expired
    });
    if history.len() == before {
        return Ok(());
    }

    if let Some(preview_cache) = app.try_state::<DecryptedPreviewCacheState>() {
        let mut cache = preview_cache.lock();
        for id in pruned_ids {
            cache.remove(id);
        }
    }

    write_history(app, &history)?;
    Ok(())
}

fn read_history(app: &AppHandle) -> Result<Vec<HistoryEntry>, AppError> {
    let store = app.store(HISTORY_STORE_FILE)?;
    Ok(store
        .get(HISTORY_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

fn read_settings(app: &AppHandle) -> Result<AppSettings, AppError> {
    let store = app.store(SETTINGS_STORE_FILE)?;
    Ok(store
        .get(SETTINGS_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default())
}

/// Writes to both the history.json and the HistoryCacheState
pub fn write_history(app: &AppHandle, history: &[HistoryEntry]) -> Result<(), AppError> {
    let store = app.store(HISTORY_STORE_FILE)?;
    store.set(HISTORY_KEY, serde_json::to_value(history)?);
    store.save()?;
    wipe_history_cache(app);
    write_history_cache(app, Arc::new(history.to_vec()));
    Ok(())
}

/// Writes to both settings.json and the SettingsState
pub fn write_settings(app: &AppHandle, settings: &AppSettings) -> Result<(), AppError> {
    let store = app.store(SETTINGS_STORE_FILE)?;
    store.set(SETTINGS_KEY, serde_json::to_value(settings)?);
    store.save()?;
    write_settings_cache(app, settings);
    Ok(())
}

// To-Do: Add doc comments explaining exactly what each state does, what's the difference
// between HistoryCacheState, DecryptedCacheState, and DecryptedPreviewCacheState.
fn manage_state(app: &mut App, settings: AppSettings) {
    let capture_paused = settings.capture_paused;
    app.manage::<VaultState>(RwLock::new(Vault::default()));
    app.manage::<SettingsState>(RwLock::new(settings));
    app.manage::<HistoryCacheState>(RwLock::new(None));
    app.manage::<DecryptedCacheState>(Mutex::new(DecryptedCache::default()));
    app.manage::<DecryptedPreviewCacheState>(Mutex::new(DecryptedPreviewCache::default()));
    app.manage::<LastHashState>(Mutex::new(None));
    app.manage::<HistoryWriteLock>(Mutex::new(()));
    app.manage::<CapturePaused>(CapturePaused(AtomicBool::new(capture_paused)));
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let settings = read_settings(app.handle())?;
            manage_state(app, settings);

            if let Err(e) = load_vault_recipient(app.handle()) {
                eprintln!("failed to load vault recipient on startup: {e}");
            }

            if let Err(e) = prune_expired(app.handle()) {
                eprintln!("failed to prune history on startup: {e}");
            }

            let handle = app.handle().clone();
            thread::spawn(move || {
                let manager = ClipboardManager::new(handle);
                let mut watcher =
                    ClipboardWatcherContext::new().expect("failed to init clipboard watcher");
                watcher.add_handler(manager);
                watcher.start_watch();
            });

            let show_item = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let capture_paused = app.state::<CapturePaused>().0.load(Ordering::Relaxed);
            let pause_item = CheckMenuItem::with_id(
                app,
                "pause",
                "Pause capture",
                true,
                capture_paused,
                None::<&str>,
            )?;
            let exit_item = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &pause_item, &exit_item])?;
            app.manage(PauseMenuItem(pause_item));

            TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Clipboard History")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                            let _ = window.unminimize();
                        }
                    }
                    "pause" => {
                        let paused_state = app.state::<CapturePaused>();
                        let paused = !paused_state.0.load(Ordering::Relaxed);
                        paused_state.0.store(paused, Ordering::Relaxed);
                        if let Some(item) = app.try_state::<PauseMenuItem>() {
                            let _ = item.0.set_checked(paused);
                        }
                        let settings = {
                            let settings_state = app.state::<SettingsState>();
                            let mut guard = settings_state.write();
                            guard.capture_paused = paused;
                            guard.clone()
                        };
                        if let Err(e) = write_settings(app, &settings) {
                            eprintln!("failed to persist capture pause: {e}");
                        }
                        if let Err(e) = app.emit("capture_paused_changed", paused) {
                            eprintln!("failed to emit: {e}");
                        }
                    }
                    "exit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                            let _ = window.unminimize();
                        }
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            load_history,
            load_settings,
            set_retention,
            set_auto_lock,
            clear_history,
            delete_from_history,
            copy_to_clipboard,
            get_full_content,
            encryption_status,
            setup_encryption,
            disable_encryption,
            unlock,
            lock,
            wipe_and_reset,
            reveal_in_directory,
            search_history,
            capture_paused,
            set_capture_paused
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
