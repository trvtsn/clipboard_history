use crate::AppError;
use age::secrecy::{ExposeSecret, SecretString};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use clipboard_history::{
    CopiedObject, EncryptionConfig, ObjectContent, ObjectFormat,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use parking_lot::RwLock;
use zeroize::{Zeroize, Zeroizing};
use std::io::{Read, Write};
use std::str::FromStr;
use std::sync::Arc;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
pub enum HistoryEntry {
    Encrypted(EncryptedRecord),
    Plain(CopiedObject),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct EncryptedRecord {
    pub id: u32,
    pub date: u64,
    pub ciphertext: String,
}

#[derive(Serialize, Deserialize)]
pub struct PlaintextPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub date: Option<u64>,
    pub content: ObjectContent,
    pub content_format: ObjectFormat,
    pub thumbnail: Option<Vec<u8>>,
    #[serde(default)]
    pub formatted_content: Option<String>,
}

#[derive(Default)]
pub struct Vault {
    pub recipient: Option<age::x25519::Recipient>,
    pub identity: Option<Arc<age::x25519::Identity>>,
}

// RwLock as we're doing more reads than writes with this.
/// Holds the values for 
pub type VaultState = RwLock<Vault>;

pub fn content_hash(content: &ObjectContent) -> String {
    let mut hasher = Sha256::new();
    match content {
        ObjectContent::Text(s) => {
            hasher.update(b"text:");
            hasher.update(s.as_bytes());
        }
        ObjectContent::Rtf(s) => {
            hasher.update(b"rtf:");
            hasher.update(s.as_bytes());
        }
        ObjectContent::Html(s) => {
            hasher.update(b"html:");
            hasher.update(s.as_bytes());
        }
        ObjectContent::Image(bytes) => {
            hasher.update(b"image:");
            hasher.update(bytes);
        }
        ObjectContent::Files(files) => {
            hasher.update(b"files:");
            for file in files {
                hasher.update(file.as_bytes());
                hasher.update(b"\0");
            }
        }
        ObjectContent::Other(name, bytes) => {
            hasher.update(b"other:");
            hasher.update(name.as_bytes());
            hasher.update(b"\0");
            hasher.update(bytes);
        }
    }
    B64.encode(hasher.finalize())
}

pub fn generate_config(password: SecretString) -> Result<(EncryptionConfig, age::x25519::Identity), AppError> {
    let identity = age::x25519::Identity::generate();
    let recipient_str = identity.to_public().to_string();
    let identity_str = identity.to_string();

    let encryptor = age::Encryptor::with_user_passphrase(password);
    let mut ciphertext = Vec::new();
    let mut writer = encryptor
        .wrap_output(&mut ciphertext)
        .map_err(|e| AppError::Crypto(e.to_string()))?;
    writer
        .write_all(identity_str.expose_secret().as_bytes())
        .map_err(|e| AppError::Crypto(e.to_string()))?;
    writer
        .finish()
        .map_err(|e| AppError::Crypto(e.to_string()))?;

    let config = EncryptionConfig {
        recipient: recipient_str,
        encrypted_identity: B64.encode(&ciphertext),
    };
    Ok((config, identity))
}

pub fn parse_recipient(recipient: &str) -> Result<age::x25519::Recipient, AppError> {
    age::x25519::Recipient::from_str(recipient)
        .map_err(|e| AppError::Crypto(format!("recipient parse: {e}")))
}

pub fn unlock_identity(
    config: &EncryptionConfig,
    password: SecretString,
) -> Result<age::x25519::Identity, AppError> {
    let ciphertext = B64
        .decode(&config.encrypted_identity)
        .map_err(|e| AppError::Crypto(format!("base64: {e}")))?;
    let decryptor = match age::Decryptor::new(&ciphertext[..])
        .map_err(|e| AppError::Crypto(e.to_string()))?
    {
        age::Decryptor::Passphrase(d) => d,
        age::Decryptor::Recipients(_) => {
            return Err(AppError::Crypto(
                "encrypted identity is not passphrase-encrypted".into(),
            ))
        }
    };
    let mut reader = decryptor
        .decrypt(&password, None)
        .map_err(|_| AppError::BadPassword)?;
    let mut identity_str = Zeroizing::new(String::new());
    reader
        .read_to_string(&mut identity_str)
        .map_err(|e| AppError::Crypto(e.to_string()))?;
    age::x25519::Identity::from_str(identity_str.trim())
        .map_err(|e| AppError::Crypto(format!("identity parse: {e}")))
}

impl EncryptedRecord {
    pub fn decrypt_with_identity(&self, identity: &age::x25519::Identity) -> Result<PlaintextPayload, AppError> {
        let ciphertext = B64
            .decode(&self.ciphertext)
            .map_err(|e| AppError::Crypto(format!("base64: {e}")))?;
        let decryptor = match age::Decryptor::new(&ciphertext[..])
            .map_err(|e| AppError::Crypto(e.to_string()))?
        {
            age::Decryptor::Recipients(d) => d,
            age::Decryptor::Passphrase(_) => {
                return Err(AppError::Crypto(
                    "ciphertext is passphrase-encrypted, not recipient-encrypted".into(),
                ))
            }
        };
        let mut plaintext = Zeroizing::new(Vec::new());
        let mut reader = decryptor
            .decrypt(std::iter::once(identity as &dyn age::Identity))
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        reader
            .read_to_end(&mut plaintext)
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        Ok(serde_json::from_slice(&plaintext)?)
    }
}

impl HistoryEntry {
    pub fn id(&self) -> u32 {
        match self {
            Self::Plain(o) => o.id,
            Self::Encrypted(e) => e.id,
        }
    }

    pub fn date(&self) -> u64 {
        match self {
            Self::Plain(o) => o.date,
            Self::Encrypted(e) => e.date,
        }
    }
}

impl PlaintextPayload {
    pub fn authoritative_id(&self, record: &EncryptedRecord) -> u32 {
        self.id.unwrap_or(record.id)
    }

    pub fn authoritative_date(&self, record: &EncryptedRecord) -> u64 {
        self.date.unwrap_or(record.date)
    }

    pub fn encrypt_to_recipient(&self, recipient: &age::x25519::Recipient) -> Result<String, AppError> {
        let plaintext = Zeroizing::new(serde_json::to_vec(self)?);
        let encryptor = age::Encryptor::with_recipients(vec![Box::new(recipient.clone())])
            .ok_or_else(|| AppError::Crypto("failed to build encryptor".into()))?;
        let mut ciphertext = Vec::new();
        let mut writer = encryptor
            .wrap_output(&mut ciphertext)
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        writer
            .write_all(&plaintext)
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        writer
            .finish()
            .map_err(|e| AppError::Crypto(e.to_string()))?;
        Ok(B64.encode(&ciphertext))
    }
}

impl Zeroize for EncryptedRecord {
    fn zeroize(&mut self) {
        self.ciphertext.zeroize();
        self.id.zeroize();
        self.date.zeroize();
    }
}

impl Zeroize for HistoryEntry {
    fn zeroize(&mut self) {
        match self {
            Self::Plain(o) => o.zeroize(),
            Self::Encrypted(e) => e.zeroize(),
        }
    }
}

impl Zeroize for PlaintextPayload {
    fn zeroize(&mut self) {
        self.content.zeroize();
        if let Some(thumbnail) = self.thumbnail.as_mut() {
            thumbnail.zeroize();
        }
        if let Some(formatted) = self.formatted_content.as_mut() {
            formatted.zeroize();
        }
    }
}
