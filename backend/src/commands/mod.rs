use crate::cache::{DecryptedCacheState, DecryptedPreviewCacheState};
use crate::crypto::{
    EncryptedRecord, HistoryEntry, PlaintextPayload, VaultState, generate_config, parse_recipient,
    unlock_identity,
};
use crate::{
    AppError, CapturePaused, HistoryWriteLock, LastHashState, PauseMenuItem, SettingsState,
    history_snapshot, wipe_history_cache, write_history, write_settings,
};
use clipboard_history::constants::WIPE_CONFIRMATION_PHRASE;
use clipboard_history::{
    AppSettings, CopiedObject, CopiedObjectPreview, EncryptionStatus, ObjectContent, ObjectFormat,
    RetentionUnit,
};
use clipboard_rs::{Clipboard, ClipboardContext, RustImageData, common::RustImage};
use rayon::prelude::*;
use secrecy::{ExposeSecret, SecretString};
use std::sync::Arc;
use std::sync::atomic::Ordering;
#[cfg(debug_assertions)]
use std::time::Instant;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_opener::OpenerExt;
use zeroize::{Zeroize, Zeroizing};

#[tauri::command]
pub async fn capture_paused(state: State<'_, CapturePaused>) -> Result<bool, AppError> {
    Ok(state.0.load(Ordering::Relaxed))
}

#[tauri::command]
pub async fn set_capture_paused(
    app: AppHandle,
    state: State<'_, CapturePaused>,
    settings_state: State<'_, SettingsState>,
    paused: bool,
) -> Result<(), AppError> {
    state.0.store(paused, Ordering::Relaxed);

    let settings = {
        let mut settings = settings_state.write();
        settings.capture_paused = paused;
        settings.clone()
    };
    write_settings(&app, &settings)?;

    if let Some(item) = app.try_state::<PauseMenuItem>() {
        let item = item.0.clone();
        let _ = app.run_on_main_thread(move || {
            let _ = item.set_checked(paused);
        });
    }

    let _ = app.emit("capture_paused_changed", paused);
    Ok(())
}

#[tauri::command]
pub async fn clear_history(
    app: AppHandle,
    vault: State<'_, VaultState>,
    history_write_lock: State<'_, HistoryWriteLock>,
    cache: State<'_, DecryptedCacheState>,
    preview_cache: State<'_, DecryptedPreviewCacheState>,
    last_hash: State<'_, LastHashState>,
) -> Result<(), AppError> {
    let _h = history_write_lock.lock();
    cache.lock().clear();
    preview_cache.lock().clear();
    *last_hash.lock() = None;

    let guarded_vault = vault.read();
    if guarded_vault.recipient.is_some() && guarded_vault.identity.is_none() {
        return Err(AppError::Locked);
    }

    write_history(&app, &[])?;

    Ok(())
}

#[tauri::command]
pub async fn copy_to_clipboard(
    app: AppHandle,
    vault: State<'_, VaultState>,
    id: u32,
    formatted: bool,
) -> Result<(), AppError> {
    let history = history_snapshot(&app)?;
    let entry = history
        .iter()
        .find(|entry| entry.id() == id)
        .ok_or_else(|| AppError::Custom(format!("No item with id: {id} in history")))?;

    let decrypted;
    let (content, content_format, formatted_content) = match entry {
        HistoryEntry::Plain(o) => (
            &o.content,
            &o.content_format,
            o.formatted_content.as_deref(),
        ),
        HistoryEntry::Encrypted(e) => {
            let guarded_vault = vault.read();
            let identity = guarded_vault.identity.as_ref().ok_or(AppError::Locked)?;
            decrypted = Zeroizing::new(e.decrypt_with_identity(identity)?);
            (
                &decrypted.content,
                &decrypted.content_format,
                decrypted.formatted_content.as_deref(),
            )
        }
    };

    let ctx = ClipboardContext::new()?;

    if formatted {
        if let Some(rich) = formatted_content {
            match content_format {
                ObjectFormat::Html => {
                    ctx.set_html(rich.to_owned())?;
                    return Ok(());
                }
                ObjectFormat::Rtf => {
                    ctx.set_rich_text(rich.to_owned())?;
                    return Ok(());
                }
                _ => {}
            }
        }
    }

    match content {
        ObjectContent::Text(s) => ctx.set_text(s.clone())?,
        ObjectContent::Rtf(s) => ctx.set_rich_text(s.clone())?,
        ObjectContent::Html(s) => ctx.set_html(s.clone())?,
        ObjectContent::Image(bytes) => {
            let img = RustImageData::from_bytes(bytes)?;
            ctx.set_image(img)?;
        }
        ObjectContent::Files(files) => {
            ctx.set_files(files.clone())?;
        }
        ObjectContent::Other(name, data) => {
            ctx.set_buffer(name, data.clone())?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn delete_from_history(
    app: AppHandle,
    vault: State<'_, VaultState>,
    history_write_lock: State<'_, HistoryWriteLock>,
    cache: State<'_, DecryptedCacheState>,
    preview_cache: State<'_, DecryptedPreviewCacheState>,
    last_hash: State<'_, LastHashState>,
    id: u32,
) -> Result<(), AppError> {
    cache.lock().remove(id);
    preview_cache.lock().remove(id);
    *last_hash.lock() = None;
    let _h = history_write_lock.lock();

    let guarded_vault = vault.read();
    if guarded_vault.recipient.is_some() && guarded_vault.identity.is_none() {
        return Err(AppError::Locked);
    }

    let mut history = Arc::unwrap_or_clone(history_snapshot(&app)?);
    let before = history.len();
    history.retain(|item| item.id() != id);
    if history.len() == before {
        return Err(AppError::Custom(format!(
            "No item with id: {id} in history"
        )));
    }

    write_history(&app, &history)
}

#[tauri::command]
pub async fn encryption_status(
    vault: State<'_, VaultState>,
    settings_state: State<'_, SettingsState>,
) -> Result<EncryptionStatus, AppError> {
    let settings = settings_state.read();
    let guarded_vault = vault.read();
    Ok(EncryptionStatus {
        enabled: settings.encryption.is_some(),
        unlocked: guarded_vault.identity.is_some(),
    })
}

#[tauri::command]
pub async fn get_full_content(
    app: AppHandle,
    vault: State<'_, VaultState>,
    decrypted_cache: State<'_, DecryptedCacheState>,
    id: u32,
) -> Result<CopiedObject, AppError> {
    #[cfg(debug_assertions)]
    let t0 = Instant::now();

    if let Some(obj) = decrypted_cache.lock().get(id) {
        #[cfg(debug_assertions)]
        eprintln!("get_full_content id={id}: HIT {:?}", t0.elapsed());
        return Ok(obj);
    }

    let history = history_snapshot(&app)?;
    let entry = history
        .iter()
        .find(|entry| entry.id() == id)
        .ok_or_else(|| AppError::Custom(format!("No item with id: {id} in history")))?;

    #[cfg(debug_assertions)]
    let t1 = Instant::now();
    let obj = match entry {
        HistoryEntry::Plain(o) => o.clone(),
        HistoryEntry::Encrypted(e) => {
            let guarded_vault = vault.read();
            let identity = guarded_vault.identity.as_ref().ok_or(AppError::Locked)?;
            let payload = e.decrypt_with_identity(identity)?;
            CopiedObject {
                id: payload.authoritative_id(e),
                date: payload.authoritative_date(e),
                content: payload.content,
                content_format: payload.content_format,
                thumbnail: payload.thumbnail,
                formatted_content: payload.formatted_content,
            }
        }
    };
    #[cfg(debug_assertions)]
    let decrypt = t1.elapsed();

    decrypted_cache.lock().insert(id, obj.clone());

    #[cfg(debug_assertions)]
    eprintln!("get_full_content id={id}: MISS decrypt+build={decrypt:?}");
    Ok(obj)
}

#[tauri::command]
pub async fn load_history(
    app: AppHandle,
    vault: State<'_, VaultState>,
    preview_cache: State<'_, DecryptedPreviewCacheState>,
    history_write_lock: State<'_, HistoryWriteLock>,
) -> Result<Vec<CopiedObjectPreview>, AppError> {
    let history = history_snapshot(&app)?;
    let identity = {
        let guarded_vault = vault.read();
        if guarded_vault.recipient.is_some() && guarded_vault.identity.is_none() {
            return Err(AppError::Locked);
        }
        guarded_vault.identity.clone()
    };

    let mut out = Vec::with_capacity(history.len());
    let mut repairs: Vec<(usize, u32, u64)> = Vec::new();

    #[cfg(debug_assertions)]
    let timer = Instant::now();

    for (idx, entry) in history.iter().enumerate() {
        if let Some(preview) = preview_cache.lock().get(entry.id()) {
            out.push(preview);
            continue;
        }

        let preview = match entry {
            HistoryEntry::Plain(o) => CopiedObjectPreview::from_full(o),
            HistoryEntry::Encrypted(encrypted_record) => {
                let identity = identity.as_ref().ok_or(AppError::Locked)?;
                let payload = encrypted_record.decrypt_with_identity(identity)?;
                let auth_id = payload.authoritative_id(encrypted_record);
                let auth_date = payload.authoritative_date(encrypted_record);

                if auth_id != encrypted_record.id || auth_date != encrypted_record.date {
                    repairs.push((idx, auth_id, auth_date));
                }

                let mut full = CopiedObject {
                    id: auth_id,
                    content: payload.content,
                    content_format: payload.content_format,
                    date: auth_date,
                    thumbnail: payload.thumbnail,
                    formatted_content: payload.formatted_content,
                };
                let preview = CopiedObjectPreview::from_full(&full);
                full.zeroize();
                preview
            }
        };

        preview_cache.lock().insert(preview.id, preview.clone());
        out.push(preview);
    }

    #[cfg(debug_assertions)]
    println!(
        "load_history: {} entries, {:?} on {} threads",
        history.len(),
        timer.elapsed(),
        rayon::current_num_threads()
    );

    if !repairs.is_empty() {
        #[cfg(debug_assertions)]
        eprintln!("Repairs: {repairs:?}");
        let _h = history_write_lock.lock();
        let mut owned = (*history).clone();
        for (idx, id, date) in repairs {
            if let Some(HistoryEntry::Encrypted(encrypted_record)) = owned.get_mut(idx) {
                encrypted_record.id = id;
                encrypted_record.date = date;
            }
        }
        write_history(&app, &owned)?;
    }

    Ok(out)
}

#[tauri::command]
pub async fn load_settings(
    settings_state: State<'_, SettingsState>,
) -> Result<AppSettings, AppError> {
    Ok(settings_state.read().clone())
}

#[tauri::command]
pub async fn lock(
    app: AppHandle,
    vault: State<'_, VaultState>,
    decrypted_cache: State<'_, DecryptedCacheState>,
    preview_cache: State<'_, DecryptedPreviewCacheState>,
) -> Result<(), AppError> {
    let mut guarded_vault = vault.write();
    guarded_vault.identity = None;

    decrypted_cache.lock().clear();
    preview_cache.lock().clear();
    wipe_history_cache(&app);

    Ok(())
}

#[tauri::command]
pub fn reveal_in_directory(app: AppHandle, path: String) -> Result<(), AppError> {
    app.opener()
        .reveal_item_in_dir(path)
        .map_err(|e| AppError::Custom(e.to_string()))
}

fn object_contains(content: &ObjectContent, terms: &[String]) -> bool {
    let content = match content {
        ObjectContent::Text(content) | ObjectContent::Html(content) | ObjectContent::Rtf(content) => content.to_lowercase(),
        ObjectContent::Files(files) => files.join("\n").to_lowercase(),
        ObjectContent::Image(_) => return false,
        ObjectContent::Other(_, _) => return false
    };

    terms.into_iter().all(|term| content.contains(term))
}

#[tauri::command]
pub async fn search_history(
    app: AppHandle,
    vault: State<'_, VaultState>,
    decrypted_cache: State<'_, DecryptedCacheState>,
    query: String,
) -> Result<Vec<u32>, AppError> {
    let history = history_snapshot(&app)?;
    let terms = query
        .to_lowercase()
        .split_whitespace()
        .map(str::to_string)
        .collect::<Vec<String>>();

    if terms.is_empty() {
        return Ok(history.iter().map(HistoryEntry::id).collect());
    }

    let identity = {
        let guarded_vault = vault.read();
        if guarded_vault.recipient.is_some() && guarded_vault.identity.is_none() {
            return Err(AppError::Locked);
        }
        guarded_vault.identity.clone()
    };

    let cache = &*decrypted_cache;

    #[cfg(debug_assertions)]
    let timer = Instant::now();

    let ids = history.par_iter().map(|entry| -> Result<Option<u32>, AppError> {
        let matched = match entry {
            HistoryEntry::Plain(o) => object_contains(&o.content, &terms),
            HistoryEntry::Encrypted(e) => {
                let hit = cache.lock().get(e.id);
                match hit {
                    Some(obj) => object_contains(&Zeroizing::new(obj).content, &terms),
                    None => {
                        #[cfg(debug_assertions)]
                        let identity = identity.as_ref().ok_or(AppError::Locked)?;
                        let payload = Zeroizing::new(e.decrypt_with_identity(identity)?);
                        object_contains(&payload.content, &terms)
                    }
                }
            }
        };

        Ok(matched.then(|| entry.id()))
    })
    .collect::<Result<Vec<_>, _>>()?;
    let ids = ids.into_iter().flatten().collect::<Vec<u32>>();

    #[cfg(debug_assertions)]
    println!(
        "search_history: {} entries, {} matched, {:?} on {} threads",
        history.len(),
        ids.len(),
        timer.elapsed(),
        rayon::current_num_threads()
    );
    
    Ok(ids)
}

#[tauri::command]
pub async fn set_auto_lock(
    app: AppHandle,
    settings_state: State<'_, SettingsState>,
    minutes: u64,
) -> Result<(), AppError> {
    let settings = {
        let mut settings = settings_state.write();
        settings.auto_lock_minutes = minutes;
        settings.clone()
    };
    write_settings(&app, &settings)
}

#[tauri::command]
pub async fn set_retention(
    app: AppHandle,
    settings_state: State<'_, SettingsState>,
    amount: u64,
    unit: RetentionUnit,
) -> Result<(), AppError> {
    let settings = {
        let mut settings = settings_state.write();
        settings.retention_amount = amount;
        settings.retention_unit = unit;
        settings.clone()
    };
    write_settings(&app, &settings)?;

    Ok(())
}

#[tauri::command]
pub async fn setup_encryption(
    app: AppHandle,
    vault: State<'_, VaultState>,
    history_write_lock: State<'_, HistoryWriteLock>,
    settings_state: State<'_, SettingsState>,
    password: SecretString,
) -> Result<(), AppError> {
    if password.expose_secret().is_empty() {
        return Err(AppError::Custom("Password must not be empty".into()));
    }

    {
        let settings = settings_state.read();
        if settings.encryption.is_some() {
            return Err(AppError::Custom("Encryption is already enabled".into()));
        }
    }

    let (config, identity) = generate_config(password)?;
    let recipient = parse_recipient(&config.recipient)?;

    let mut settings = settings_state.write();
    if settings.encryption.is_some() {
        return Err(AppError::Custom("Encryption is already enabled".into()));
    }

    let _h = history_write_lock.lock();
    let mut guarded_vault = vault.write();

    let history = Arc::unwrap_or_clone(history_snapshot(&app)?);
    
    #[cfg(debug_assertions)]
    let (timer, total_entries) = (Instant::now(), history.len());

    let migrated = history.into_par_iter().map(|entry| -> Result<HistoryEntry, AppError> {
        match entry {
            HistoryEntry::Encrypted(encrypted_record) => {
                Ok(HistoryEntry::Encrypted(encrypted_record))
            }
            HistoryEntry::Plain(copied_object) => {
                let payload = Zeroizing::new(PlaintextPayload {
                    id: Some(copied_object.id),
                    date: Some(copied_object.date),
                    content: copied_object.content,
                    content_format: copied_object.content_format,
                    thumbnail: copied_object.thumbnail,
                    formatted_content: copied_object.formatted_content,
                });
                let ciphertext = payload.encrypt_to_recipient(&recipient)?;
                Ok(HistoryEntry::Encrypted(EncryptedRecord {
                    id: copied_object.id,
                    date: copied_object.date,
                    ciphertext,
                }))
            }
        }
    }).collect::<Result<Vec<_>, _>>()?;

    #[cfg(debug_assertions)]
    println!(
        "setup_encryption: {} entries, {:?} on {} threads",
        total_entries,
        timer.elapsed(),
        rayon::current_num_threads()
    );

    write_history(&app, &migrated)?;

    settings.encryption = Some(config);
    let settings_snapshot = settings.clone();
    drop(settings);
    write_settings(&app, &settings_snapshot)?;

    guarded_vault.recipient = Some(recipient);
    guarded_vault.identity = Some(Arc::new(identity));
    Ok(())
}

#[tauri::command]
pub async fn disable_encryption(
    app: AppHandle,
    vault: State<'_, VaultState>,
    history_write_lock: State<'_, HistoryWriteLock>,
    settings_state: State<'_, SettingsState>,
    password: SecretString,
) -> Result<(), AppError> {
    let cfg = {
        let settings = settings_state.read();
        settings
            .encryption
            .clone()
            .ok_or_else(|| AppError::Custom("Encryption is not enabled".into()))?
    };

    let identity = unlock_identity(&cfg, password)?;

    let mut settings = settings_state.write();
    let Some(ref current_cfg) = settings.encryption else {
        return Err(AppError::Custom("Encryption is not enabled".into()));
    };

    if cfg != *current_cfg {
        return Err(AppError::EncryptionReconfigured);
    }

    let _h = history_write_lock.lock();
    let mut guarded_vault = vault.write();

    let history = Arc::unwrap_or_clone(history_snapshot(&app)?);

    #[cfg(debug_assertions)]
    let (timer, total_entries) = (Instant::now(), history.len());

    let mut migrated = history.into_par_iter().map(|entry| -> Result<HistoryEntry, AppError> {
        match entry {
            HistoryEntry::Plain(o) => Ok(HistoryEntry::Plain(o)),
            HistoryEntry::Encrypted(e) => {
                let payload = Zeroizing::new(e.decrypt_with_identity(&identity)?);
                Ok(HistoryEntry::Plain(CopiedObject {
                    id: payload.authoritative_id(&e),
                    date: payload.authoritative_date(&e),
                    content: payload.content.clone(),
                    content_format: payload.content_format.clone(),
                    thumbnail: payload.thumbnail.clone(),
                    formatted_content: payload.formatted_content.clone(),
                }))
            }
        }
    })
    .collect::<Result<Vec<_>, _>>()?;

    #[cfg(debug_assertions)]
    println!(
        "disable_encryption: {} entries, {:?} on {} threads",
        total_entries,
        timer.elapsed(),
        rayon::current_num_threads()
    );

    write_history(&app, &migrated)?;
    migrated.iter_mut().for_each(Zeroize::zeroize);

    settings.encryption = None;
    let settings_snapshot = settings.clone();
    drop(settings);
    write_settings(&app, &settings_snapshot)?;

    guarded_vault.recipient = None;
    guarded_vault.identity = None;
    Ok(())
}

#[tauri::command]
pub async fn unlock(
    vault: State<'_, VaultState>,
    settings_state: State<'_, SettingsState>,
    password: SecretString,
) -> Result<(), AppError> {
    let settings = settings_state.read();
    let cfg = settings
        .encryption
        .clone()
        .ok_or_else(|| AppError::Custom("Encryption is not enabled".into()))?;
    drop(settings);
    let identity = unlock_identity(&cfg, password)?;
    let recipient = parse_recipient(&cfg.recipient)?;
    let mut guarded_vault = vault.write();
    guarded_vault.recipient = Some(recipient);
    guarded_vault.identity = Some(Arc::new(identity));
    Ok(())
}

#[tauri::command]
pub async fn wipe_and_reset(
    app: AppHandle,
    vault: State<'_, VaultState>,
    history_write_lock: State<'_, HistoryWriteLock>,
    settings_state: State<'_, SettingsState>,
    decrypted_cache: State<'_, DecryptedCacheState>,
    preview_cache: State<'_, DecryptedPreviewCacheState>,
    last_hash: State<'_, LastHashState>,
    confirmation: String,
    new_password: Option<SecretString>,
) -> Result<(), AppError> {
    if confirmation != WIPE_CONFIRMATION_PHRASE {
        return Err(AppError::Custom(
            "Confirmation phrase required to wipe history".into(),
        ));
    }

    // To be honest I don't really like doing blocks like this, I think they just look kinda ugly,
    // but I will still leave this here for now as this is generally sufficient enough. The goal
    // is to make the locks not persist even during the KDF's of setup_encryption.
    {
        let mut settings = settings_state.write();
        let _h = history_write_lock.lock();
        let mut guarded_vault = vault.write();

        write_history(&app, &[])?;
        decrypted_cache.lock().clear();
        preview_cache.lock().clear();
        *last_hash.lock() = None;

        settings.encryption = None;
        let settings_snapshot = settings.clone();
        drop(settings);
        write_settings(&app, &settings_snapshot)?;

        guarded_vault.recipient = None;
        guarded_vault.identity = None;
    }

    if let Some(pw) = new_password
        && !pw.expose_secret().is_empty()
    {
        setup_encryption(app, vault, history_write_lock, settings_state, pw).await?;
    }
    Ok(())
}
