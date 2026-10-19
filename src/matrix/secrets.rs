//! Secret storage via the freedesktop Secret Service (oo7; falls back to
//! oo7's encrypted file keyring when no service is running).
//!
//! Fallback policy: if no secure store can be opened, secrets are NOT
//! persisted. The user must log in again next start. Credentials are never
//! written to plaintext config files.

use oo7::{Keyring, Secret};
use thiserror::Error;

const APP_ID: &str = "com.matrixmessenger.Msm";

#[derive(Debug, Error)]
pub enum SecretError {
    #[error("secure credential store unavailable: {0}")]
    Unavailable(String),
}

fn err(e: impl std::fmt::Display) -> SecretError {
    SecretError::Unavailable(e.to_string())
}

pub async fn save(kind: &str, text: &str) -> Result<(), SecretError> {
    let ring = Keyring::new().await.map_err(err)?;
    ring.create_item(
        "Matrix Messenger session",
        &[("application", APP_ID), ("kind", kind)],
        Secret::text(text),
        true,
    )
    .await
    .map_err(err)
}

pub async fn load(kind: &str) -> Result<Option<String>, SecretError> {
    let ring = Keyring::new().await.map_err(err)?;
    let items = ring
        .search_items(&[("application", APP_ID), ("kind", kind)])
        .await
        .map_err(err)?;
    match items.first() {
        Some(item) => {
            let s = item.secret().await.map_err(err)?;
            Ok(Some(String::from_utf8_lossy(s.as_bytes()).into_owned()))
        }
        None => Ok(None),
    }
}

pub async fn delete(kind: &str) -> Result<(), SecretError> {
    let ring = Keyring::new().await.map_err(err)?;
    ring.delete(&[("application", APP_ID), ("kind", kind)])
        .await
        .map_err(err)
}
