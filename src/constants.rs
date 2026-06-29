pub const WIPE_CONFIRMATION_PHRASE: &str = "WIPE_EVERYTHING";
pub const TEXT_PREVIEW_MAX_CHARS: usize = 200;
pub const SETTINGS_STORE_FILE: &str = "settings.json";
pub const HISTORY_STORE_FILE: &str = "history.json";
pub const HISTORY_KEY: &str = "history";
pub const SETTINGS_KEY: &str = "settings";

pub mod cache {
    pub const TOTAL_MAX_BYTES: usize = 8 * 1024 * 1024;
    pub const PER_ITEM_MAX_BYTES: usize = 2 * 1024 * 1024;
    pub const MAX_ITEMS: usize = 100;
}