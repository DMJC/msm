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

//! Security status, cross-signing / recovery setup, and device listing.
//!
//! Recovery key + key backup are what let a *new* device read older encrypted
//! history. Secrets (recovery key, password) are never logged.

use matrix_sdk::Client;
use matrix_sdk::encryption::recovery::RecoveryState;
use matrix_sdk::ruma::api::client::uiaa::{AuthData, Password, UserIdentifier};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SecurityError {
    #[error("not logged in")]
    NotLoggedIn,
    #[error("Wrong password, or the server refused to set up cross-signing: {0}")]
    Auth(String),
    #[error("{0}")]
    Matrix(String),
}

fn mx(e: impl std::fmt::Display) -> SecurityError {
    SecurityError::Matrix(e.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recovery {
    /// Secret storage + backup are set up and this device has the secrets.
    Enabled,
    Disabled,
    /// Set up on another device but this device lacks the secrets: enter the recovery key.
    Incomplete,
    Unknown,
}

impl Recovery {
    pub fn label(self) -> &'static str {
        match self {
            Recovery::Enabled => "Recovery is on",
            Recovery::Disabled => "Recovery is not set up",
            Recovery::Incomplete => "Recovery key needed on this device",
            Recovery::Unknown => "Checking…",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecurityStatus {
    pub device_id: String,
    pub cross_signing_ready: bool,
    pub recovery: Recovery,
    pub backup_enabled: bool,
    pub this_device_verified: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    pub device_id: String,
    pub name: Option<String>,
    pub verified: bool,
    pub is_current: bool,
}

pub async fn status(client: &Client) -> SecurityStatus {
    let enc = client.encryption();
    let own = enc.get_own_device().await.ok().flatten();
    let recovery = match enc.recovery().state() {
        RecoveryState::Enabled => Recovery::Enabled,
        RecoveryState::Disabled => Recovery::Disabled,
        RecoveryState::Incomplete => Recovery::Incomplete,
        _ => Recovery::Unknown,
    };
    SecurityStatus {
        device_id: client
            .device_id()
            .map(|d| d.to_string())
            .unwrap_or_default(),
        cross_signing_ready: enc
            .cross_signing_status()
            .await
            .is_some_and(|s| s.has_master && s.has_self_signing && s.has_user_signing),
        recovery,
        backup_enabled: enc.backups().are_enabled().await,
        // `is_verified()` is always true for our own device (locally trusted); cross-signing is what matters.
        this_device_verified: own.is_some_and(|d| d.is_cross_signed_by_owner()),
    }
}

/// All of our own devices, current device first.
pub async fn own_devices(client: &Client) -> Result<Vec<DeviceInfo>, SecurityError> {
    let me = client.user_id().ok_or(SecurityError::NotLoggedIn)?;
    let current = client.device_id().map(|d| d.to_string());
    let mut out: Vec<DeviceInfo> = client
        .encryption()
        .get_user_devices(me)
        .await
        .map_err(mx)?
        .devices()
        .map(|d| {
            let is_current = current.as_deref() == Some(d.device_id().as_str());
            DeviceInfo {
                device_id: d.device_id().to_string(),
                name: d.display_name().map(str::to_owned),
                // Our own device is always locally trusted; only cross-signing says anything.
                verified: if is_current {
                    d.is_cross_signed_by_owner()
                } else {
                    d.is_verified()
                },
                is_current,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.is_current
            .cmp(&a.is_current)
            .then(a.device_id.cmp(&b.device_id))
    });
    Ok(out)
}

/// Set up cross-signing (if needed) and recovery, returning the recovery key.
/// The key is shown to the user once and must be kept; it is not stored by the app.
/// `password` answers the server's interactive-auth challenge for uploading cross-signing keys.
pub async fn setup_recovery(client: &Client, password: &str) -> Result<String, SecurityError> {
    let user = client
        .user_id()
        .ok_or(SecurityError::NotLoggedIn)?
        .to_owned();
    let enc = client.encryption();

    if let Err(first) = enc.bootstrap_cross_signing_if_needed(None).await {
        let Some(info) = first.as_uiaa_response() else {
            return Err(mx(first));
        };
        let mut pw = Password::new(UserIdentifier::from(user), password.to_owned());
        pw.session = info.session.clone();
        enc.bootstrap_cross_signing(Some(AuthData::Password(pw)))
            .await
            .map_err(|e| SecurityError::Auth(e.to_string()))?;
    }
    enc.recovery().enable().await.map_err(mx)
}

/// Replace the recovery key (this device must hold the secrets). The old key stops working.
pub async fn reset_recovery_key(client: &Client) -> Result<String, SecurityError> {
    client.encryption().recovery().reset_key().await.map_err(mx)
}

/// Throw away this account's cross-signing identity and make a new one. Everyone
/// who verified you must verify you again. If recovery was on, its key is
/// replaced too (the old secrets stored under it are stale); the new key is returned.
/// `password` answers the server's interactive-auth challenge. A failed attempt
/// (wrong password) leaves this device's local keys out of step with the server
/// until the reset is retried successfully.
pub async fn reset_identity(
    client: &Client,
    password: &str,
) -> Result<Option<String>, SecurityError> {
    use matrix_sdk::encryption::CrossSigningResetAuthType;
    let user = client
        .user_id()
        .ok_or(SecurityError::NotLoggedIn)?
        .to_owned();
    let enc = client.encryption();
    let was_enabled = status(client).await.recovery != Recovery::Disabled;
    if let Some(handle) = enc.reset_cross_signing().await.map_err(mx)? {
        match handle.auth_type() {
            CrossSigningResetAuthType::Uiaa(info) => {
                let mut pw = Password::new(UserIdentifier::from(user), password.to_owned());
                pw.session = info.session.clone();
                handle
                    .auth(Some(AuthData::Password(pw)))
                    .await
                    .map_err(|e| SecurityError::Auth(e.to_string()))?;
            }
            CrossSigningResetAuthType::OAuth(info) => {
                return Err(SecurityError::Matrix(format!(
                    "this account signs in through your provider; approve the reset at {}",
                    info.approval_url
                )));
            }
        }
    }
    if was_enabled {
        return Ok(Some(enc.recovery().reset_key().await.map_err(mx)?));
    }
    Ok(None)
}

/// Delete secret storage and the key backup from the server. Existing devices keep
/// their local keys; new devices will no longer be able to read older history.
pub async fn disable_recovery(client: &Client) -> Result<(), SecurityError> {
    client.encryption().recovery().disable().await.map_err(mx)
}

/// Unlock secret storage with a recovery key so this device gets the backup key,
/// cross-signing keys and can decrypt older history.
pub async fn recover(client: &Client, recovery_key: &str) -> Result<(), SecurityError> {
    client
        .encryption()
        .recovery()
        .recover(recovery_key.trim())
        .await
        .map_err(|e| SecurityError::Matrix(format!("could not use that recovery key: {e}")))
}
