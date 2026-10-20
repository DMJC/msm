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

//! Login, session persistence/restoration and logout.
//!
//! The SDK SQLite store (state + crypto) lives under the XDG data dir and is
//! encrypted with a random passphrase. Tokens and that passphrase are kept in
//! the secret store only (see `secrets`).

use std::path::{Path, PathBuf};

use matrix_sdk::authentication::matrix::MatrixSession;
use matrix_sdk::{Client, ClientBuildError, store::RoomLoadSettings};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::secrets;

const SESSION_KIND: &str = "session";
const DEVICE_NAME: &str = "Matrix Messenger (Linux)";

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Please enter a homeserver address (for example matrix.org).")]
    EmptyHomeserver,
    #[error("Please enter your username and password.")]
    EmptyCredentials,
    #[error("Could not reach or recognise that homeserver: {0}")]
    Homeserver(String),
    #[error("Login failed: {0}")]
    Login(String),
    #[error("Could not access local session data: {0}")]
    Storage(String),
    #[error("Your session has expired or was revoked; please log in again.")]
    SessionInvalid,
}

impl From<ClientBuildError> for AuthError {
    fn from(e: ClientBuildError) -> Self {
        AuthError::Homeserver(e.to_string())
    }
}

#[derive(Serialize, Deserialize)]
struct StoredSession {
    homeserver_url: String,
    db_passphrase: String,
    session: MatrixSession,
}

/// Accepts `matrix.org`, `https://matrix.example.com`, or `http://localhost:8008`.
/// Returns the string handed to the SDK's discovery (server name or full URL).
pub fn normalize_homeserver(input: &str) -> Result<String, AuthError> {
    let t = input.trim().trim_end_matches('/');
    if t.is_empty() {
        return Err(AuthError::EmptyHomeserver);
    }
    if t.contains(char::is_whitespace) {
        return Err(AuthError::Homeserver("address contains spaces".into()));
    }
    Ok(t.to_owned())
}

pub fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "matrixmessenger", "Msm")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from(".msm-data"))
}

fn store_dir(base: &Path) -> PathBuf {
    base.join("matrix-store")
}

fn random_passphrase() -> Result<String, AuthError> {
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| AuthError::Storage(e.to_string()))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

async fn build_client(
    homeserver: &str,
    base: &Path,
    passphrase: &str,
) -> Result<Client, AuthError> {
    std::fs::create_dir_all(store_dir(base)).map_err(|e| AuthError::Storage(e.to_string()))?;
    let b = Client::builder().sqlite_store(store_dir(base), Some(passphrase));
    // A full URL skips discovery; a bare server name uses .well-known discovery.
    let b = if homeserver.contains("://") {
        b.homeserver_url(homeserver)
    } else {
        b.server_name_or_homeserver_url(homeserver)
    };
    Ok(b.build().await?)
}

/// Log in with a password and persist the session securely.
pub async fn login(
    homeserver: &str,
    user: &str,
    password: &str,
    base: &Path,
) -> Result<Client, AuthError> {
    let (client, json) = login_unsaved(homeserver, user, password, base).await?;
    if let Err(e) = secrets::save(SESSION_KIND, &json).await {
        // Policy: no secure store => stay logged in for this run only.
        tracing::warn!("session will not persist: {e}");
    }
    Ok(client)
}

/// Log in without touching the secret store. Returns the serialised session
/// (contains tokens: handle as a secret).
pub async fn login_unsaved(
    homeserver: &str,
    user: &str,
    password: &str,
    base: &Path,
) -> Result<(Client, String), AuthError> {
    let homeserver = normalize_homeserver(homeserver)?;
    if user.trim().is_empty() || password.is_empty() {
        return Err(AuthError::EmptyCredentials);
    }
    let passphrase = random_passphrase()?;
    // A fresh login gets a fresh store: drop any stale one from another account.
    let _ = std::fs::remove_dir_all(store_dir(base));
    let client = build_client(&homeserver, base, &passphrase).await?;
    client
        .matrix_auth()
        .login_username(user.trim(), password)
        .initial_device_display_name(DEVICE_NAME)
        .await
        .map_err(|e| AuthError::Login(e.to_string()))?;
    let session = client
        .matrix_auth()
        .session()
        .ok_or_else(|| AuthError::Login("server returned no session".into()))?;
    let stored = StoredSession {
        homeserver_url: client.homeserver().to_string(),
        db_passphrase: passphrase,
        session,
    };
    let json = serde_json::to_string(&stored).map_err(|e| AuthError::Storage(e.to_string()))?;
    Ok((client, json))
}

/// Restore a previously saved session. `Ok(None)` if there is none.
pub async fn restore(base: &Path) -> Result<Option<Client>, AuthError> {
    let json = match secrets::load(SESSION_KIND).await {
        Ok(Some(j)) => j,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::warn!("cannot read saved session: {e}");
            return Ok(None);
        }
    };
    let stored: StoredSession = match serde_json::from_str(&json) {
        Ok(s) => s,
        Err(_) => {
            // Corrupt entry: discard rather than loop on it.
            let _ = secrets::delete(SESSION_KIND).await;
            return Ok(None);
        }
    };
    let client = build_client(&stored.homeserver_url, base, &stored.db_passphrase).await?;
    client
        .matrix_auth()
        .restore_session(stored.session, RoomLoadSettings::default())
        .await
        .map_err(|e| AuthError::Storage(e.to_string()))?;
    Ok(Some(client))
}

/// Log out on the server (best effort) and wipe local session secrets and store.
pub async fn logout(client: &Client, base: &Path) {
    if let Err(e) = client.logout().await {
        tracing::warn!("server logout failed (continuing local cleanup): {e}");
    }
    let _ = secrets::delete(SESSION_KIND).await;
    let _ = std::fs::remove_dir_all(store_dir(base));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_homeserver_input() {
        assert_eq!(
            normalize_homeserver("  matrix.org/ ").unwrap(),
            "matrix.org"
        );
        assert_eq!(
            normalize_homeserver("http://localhost:8008/").unwrap(),
            "http://localhost:8008"
        );
        assert!(matches!(
            normalize_homeserver("  "),
            Err(AuthError::EmptyHomeserver)
        ));
        assert!(normalize_homeserver("a b").is_err());
    }

    #[test]
    fn passphrase_is_random_hex() {
        let a = random_passphrase().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, random_passphrase().unwrap());
    }
}
