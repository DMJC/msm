// Copyright (C) 2026 James Carthew
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

//! Secret storage via the freedesktop Secret Service (oo7; falls back to
//! oo7's encrypted file keyring when no service is running).
//!
//! Fallback policy: if no secure store can be opened, secrets are NOT
//! persisted. The user must log in again next start. Credentials are never
//! written to plaintext config files.

use oo7::{Keyring, Secret};
use thiserror::Error;

const DEFAULT_APP_ID: &str = "com.matrixmessenger.Msm";

static NAMESPACE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Use a different secret-store namespace for this process. Tests must call this
/// before touching sessions so they can never read, overwrite or delete a real
/// user's saved login. Has no effect once secrets have been accessed.
pub fn set_namespace(ns: &str) {
    let _ = NAMESPACE.set(ns.to_owned());
}

fn app_id() -> &'static str {
    NAMESPACE
        .get()
        .map(String::as_str)
        .unwrap_or(DEFAULT_APP_ID)
}

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
        &[("application", app_id()), ("kind", kind)],
        Secret::text(text),
        true,
    )
    .await
    .map_err(err)
}

pub async fn load(kind: &str) -> Result<Option<String>, SecretError> {
    let ring = Keyring::new().await.map_err(err)?;
    let items = ring
        .search_items(&[("application", app_id()), ("kind", kind)])
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
    ring.delete(&[("application", app_id()), ("kind", kind)])
        .await
        .map_err(err)
}
