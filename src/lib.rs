pub mod constants;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use zeroize::Zeroize;

use crate::constants::TEXT_PREVIEW_MAX_CHARS;

#[derive(Clone, Debug, Error, Serialize, Deserialize, PartialEq)]
pub enum AppError {
    #[error("An error occurred: {0}")]
    Custom(String),
    #[error("An error occurred with the Tauri Store plugin: {0}")]
    TauriStore(String),
    #[error("An error occurred with clipboard-rs: {0}")]
    ClipboardRs(String),
    #[error("An error occurred with serde_json: {0}")]
    SerdeJson(String),
    #[error("History is locked. Enter the password to unlock.")]
    Locked,
    #[error("Incorrect password.")]
    BadPassword,
    #[error("A cryptographic error occurred: {0}")]
    Crypto(String),
    #[error("An error occurred with Tauri's managed state system")]
    TauriState,
    #[error("Encryption configuration changed during an operation")]
    EncryptionReconfigured
}

#[cfg(not(target_arch = "wasm32"))]
impl From<tauri_plugin_store::Error> for AppError {
    fn from(value: tauri_plugin_store::Error) -> Self {
        Self::TauriStore(value.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<serde_json::Error> for AppError {
    fn from(value: serde_json::Error) -> Self {
        Self::SerdeJson(value.to_string())
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Box<dyn std::error::Error + Send + Sync + 'static>> for AppError {
    fn from(value: Box<dyn std::error::Error + Send + Sync + 'static>) -> Self {
        Self::ClipboardRs(value.to_string())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct CopiedObject {
    pub id: u32,
    pub content: ObjectContent,
    pub content_format: ObjectFormat,
    pub date: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<Vec<u8>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub formatted_content: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub enum ObjectContent {
    Text(String),
    Rtf(String),
    Html(String),
    Image(Vec<u8>),
    Files(Vec<String>),
    Other(String, Vec<u8>),
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum ObjectFormat {
    Text,
    Rtf,
    Html,
    Image,
    Files,
    Other(String),
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct CopiedObjectPreview {
    pub id: u32,
    pub content_format: ObjectFormat,
    pub date: u64,
    pub preview: PreviewContent,
    #[serde(default)]
    pub has_formatting: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum PreviewContent {
    Text(String),
    Rtf(String),
    Html(String),
    Image(Vec<u8>),
    Files(Vec<String>),
    Other(String, u64),
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub enum RetentionUnit {
    Minutes,
    Hours,
    #[default]
    Days,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct AppSettings {
    // #[serde(alias = "retention_duration_days")]
    pub retention_amount: u64,
    pub retention_unit: RetentionUnit,
    pub auto_lock_minutes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encryption: Option<EncryptionConfig>,
    #[serde(default)]
    pub capture_paused: bool
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct EncryptionConfig {
    pub recipient: String,
    pub encrypted_identity: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct EncryptionStatus {
    pub enabled: bool,
    pub unlocked: bool,
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut out = text.chars().take(max).collect::<String>();
        out.push_str("...");
        out
    }
}

impl RetentionUnit {
    pub fn ms_per_unit(self) -> u64 {
        match self {
            Self::Minutes => 60_000,
            Self::Hours => 3_600_000,
            Self::Days => 86_400_000,
        }
    }
}

impl CopiedObject {
    pub fn object_size(&self) -> usize {
        let content_length = match &self.content {
            ObjectContent::Text(string) | ObjectContent::Rtf(string) | ObjectContent::Html(string) => string.len(),
            ObjectContent::Image(bytes) => bytes.len(),
            ObjectContent::Files(files) => files.iter().map(String::len).sum(),
            ObjectContent::Other(name, bytes) => name.len() + bytes.len(),
        };
        let thumbnail_length = self.thumbnail.as_ref().map_or(0, Vec::len);
        let formatted_content_length = self.formatted_content.as_ref().map_or(0, String::len);

        content_length + thumbnail_length + formatted_content_length
    }
}

impl CopiedObjectPreview {
    pub fn object_size(&self) -> usize {
        match &self.preview {
            PreviewContent::Text(string) | PreviewContent::Rtf(string) | PreviewContent::Html(string) => string.len(),
            PreviewContent::Image(bytes) => bytes.len(),
            PreviewContent::Files(files) => files.iter().map(String::len).sum(),
            PreviewContent::Other(name, _) => name.len(),
        }
    }

    pub fn from_full(full: &CopiedObject) -> Self {
        let preview = match &full.content {
            ObjectContent::Text(s) => PreviewContent::Text(truncate(s, TEXT_PREVIEW_MAX_CHARS)),
            ObjectContent::Rtf(s) => PreviewContent::Rtf(truncate(s, TEXT_PREVIEW_MAX_CHARS)),
            ObjectContent::Html(s) => PreviewContent::Html(truncate(s, TEXT_PREVIEW_MAX_CHARS)),
            ObjectContent::Image(_) => {
                PreviewContent::Image(full.thumbnail.clone().unwrap_or_default())
            }
            ObjectContent::Files(fs) => PreviewContent::Files(fs.clone()),
            ObjectContent::Other(name, b) => PreviewContent::Other(name.clone(), b.len() as u64),
        };
        Self {
            id: full.id,
            content_format: full.content_format.clone(),
            date: full.date,
            preview,
            has_formatting: full.formatted_content.is_some(),
        }
    }
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            retention_amount: 0,
            retention_unit: RetentionUnit::default(),
            auto_lock_minutes: 5,
            encryption: None,
            capture_paused: false
        }
    }
}

impl Zeroize for CopiedObject {
    fn zeroize(&mut self) {
        self.content.zeroize();
        if let Some(bytes) = self.thumbnail.as_mut() {
            bytes.zeroize();
        }
        if let Some(string) = self.formatted_content.as_mut() {
            string.zeroize();
        }
    }
}

impl Zeroize for ObjectContent {
    fn zeroize(&mut self) {
        match self {
            ObjectContent::Text(text) => text.zeroize(),
            ObjectContent::Rtf(rtf) => rtf.zeroize(),
            ObjectContent::Html(html) => html.zeroize(),
            ObjectContent::Image(images) => images.zeroize(),
            ObjectContent::Files(files) => files.zeroize(),
            ObjectContent::Other(object_type, bytes) => {
                object_type.zeroize();
                bytes.zeroize();
            },
        }
    }
}

impl Zeroize for CopiedObjectPreview {
    fn zeroize(&mut self) {
        self.preview.zeroize();
        self.date.zeroize();
        self.content_format = ObjectFormat::Other(String::new());
        self.id.zeroize();
        self.has_formatting.zeroize();
    }
}

impl Zeroize for PreviewContent {
    fn zeroize(&mut self) {
        match self {
            PreviewContent::Text(text) => text.zeroize(),
            PreviewContent::Rtf(rtf) => rtf.zeroize(),
            PreviewContent::Html(html) => html.zeroize(),
            PreviewContent::Image(images) => images.zeroize(),
            PreviewContent::Files(files) => files.zeroize(),
            PreviewContent::Other(object_type, bytes) => {
                object_type.zeroize();
                bytes.zeroize();
            },
        }
    }
}
