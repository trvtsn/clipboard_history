use clipboard_history::{
    AppSettings, CopiedObject, CopiedObjectPreview,
    constants::cache::{MAX_ITEMS, PER_ITEM_MAX_BYTES, TOTAL_MAX_BYTES},
};
use parking_lot::{Mutex, RwLock};
use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
};
use tauri::{AppHandle, Manager};
use zeroize::Zeroize;

use crate::{SettingsState, crypto::HistoryEntry};

// RwLock as we're doing more reads than writes with this.
/// Stores the entries from the history.json into memory.
/// Fetched once (from disk) on program startup, and once every unlock.
/// Gets freed and zeroized on lock.
pub type HistoryCacheState = RwLock<Option<Arc<Vec<HistoryEntry>>>>;

// Mutex as we're doing more writes than reads with this.
/// Stores decrypted entries from history.
/// Populates on every `get_full_content` call. Used to decrease loading times
/// and avoid re-decrypting from disk whenever the full content of a history entry is queried.
/// Gets freed and zeroized on lock.
pub type DecryptedCacheState = Mutex<DecryptedCache>;

// Mutex as we're doing more writes than reads with this.
/// Stores the decrypted previews (truncated text, minimized images).
/// Populates on every `load_history` call. Used to decrease loading times
/// and avoid re-decrypting from disk whenever the history table is rendered.
/// Gets freed and zeroized on lock.
pub type DecryptedPreviewCacheState = Mutex<DecryptedPreviewCache>;

#[derive(Default)]
pub struct DecryptedCache {
    map: HashMap<u32, (CopiedObject, usize)>,
    order: VecDeque<u32>,
    bytes: usize,
}

#[derive(Default)]
pub struct DecryptedPreviewCache {
    map: HashMap<u32, (CopiedObjectPreview, usize)>,
    order: VecDeque<u32>,
    bytes: usize,
}

impl DecryptedCache {
    pub fn get(&mut self, id: u32) -> Option<CopiedObject> {
        let object = self.map.get(&id)?.0.clone();
        self.touch(id);
        Some(object)
    }

    pub fn insert(&mut self, id: u32, object: CopiedObject) {
        let size = object.object_size();
        if size > PER_ITEM_MAX_BYTES {
            return;
        }
        self.remove(id);
        self.map.insert(id, (object, size));
        self.order.push_back(id);
        self.bytes += size;
        self.evict();
    }

    pub fn remove(&mut self, id: u32) {
        if let Some((mut obj, size)) = self.map.remove(&id) {
            self.bytes -= size;
            self.order.retain(|entry_id| *entry_id != id);
            obj.zeroize();
        }
    }

    pub fn clear(&mut self) {
        for (_, (mut object, _)) in self.map.drain() {
            object.zeroize();
        }
        self.order.clear();
        self.bytes = 0;
    }

    fn touch(&mut self, id: u32) {
        if let Some(position) = self.order.iter().position(|entry_id| *entry_id == id) {
            self.order.remove(position);
            self.order.push_back(id);
        }
    }

    fn evict(&mut self) {
        while self.bytes > TOTAL_MAX_BYTES || self.map.len() > MAX_ITEMS {
            let Some(id) = self.order.pop_front() else { break };
            if let Some((mut object, size)) = self.map.remove(&id) {
                self.bytes -= size;
                object.zeroize();
            }
        }
    }
}

impl DecryptedPreviewCache {
    pub fn get(&mut self, id: u32) -> Option<CopiedObjectPreview> {
        let preview = self.map.get(&id)?.0.clone();
        self.touch(id);
        Some(preview)
    }

    pub fn insert(&mut self, id: u32, preview: CopiedObjectPreview) {
        let size = preview.object_size();
        self.remove(id);
        self.map.insert(id, (preview, size));
        self.order.push_back(id);
        self.bytes += size;
        self.evict();
    }

    pub fn remove(&mut self, id: u32) {
        if let Some((mut preview, size)) = self.map.remove(&id) {
            self.bytes -= size;
            self.order.retain(|x| *x != id);
            preview.preview.zeroize();
        }
    }

    pub fn clear(&mut self) {
        for (_, (mut preview, _)) in self.map.drain() {
            preview.preview.zeroize();
        }
        self.order.clear();
        self.bytes = 0;
    }

    fn touch(&mut self, id: u32) {
        if let Some(position) = self.order.iter().position(|x| *x == id) {
            self.order.remove(position);
            self.order.push_back(id);
        }
    }

    fn evict(&mut self) {
        while self.bytes > TOTAL_MAX_BYTES {
            let Some(id) = self.order.pop_front() else { break };
            if let Some((mut preview, size)) = self.map.remove(&id) {
                self.bytes -= size;
                preview.preview.zeroize();
            }
        }
    }
}

pub fn write_history_cache(app: &AppHandle, history: Arc<Vec<HistoryEntry>>) {
    if let Some(cache) = app.try_state::<HistoryCacheState>() {
        *cache.write() = Some(history);
    }
}

pub fn wipe_history_cache(app: &AppHandle) {
    if let Some(cache) = app.try_state::<HistoryCacheState>() {
        let taken = cache.write().take();
        if let Some(mut entries) = taken.and_then(Arc::into_inner) {
            entries.iter_mut().for_each(Zeroize::zeroize);
        }
    }
}

pub fn write_settings_cache(app: &AppHandle, settings: &AppSettings) {
    if let Some(cache) = app.try_state::<SettingsState>() {
        *cache.write() = settings.clone();
    }
}
