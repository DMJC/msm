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

//! Blocking people uses Matrix's own ignore list (`m.ignored_user_list` account
//! data), so it applies to every client on the account and the SDK drops the
//! ignored user's events.

use matrix_sdk::Client;
use matrix_sdk::ruma::UserId;
use matrix_sdk::ruma::events::ignored_user_list::{IgnoredUser, IgnoredUserListEventContent};

#[derive(Debug, thiserror::Error)]
pub enum IgnoreError {
    #[error("not a valid Matrix ID: {0}")]
    InvalidUser(String),
    #[error("{0}")]
    Matrix(String),
}

fn parse(mxid: &str) -> Result<&UserId, IgnoreError> {
    <&UserId>::try_from(mxid.trim()).map_err(|_| IgnoreError::InvalidUser(mxid.to_owned()))
}

/// The accounts currently blocked. Asks the server, so a change just made is
/// visible before the next sync echoes it; falls back to the synced copy.
pub async fn blocked(client: &Client) -> Vec<String> {
    let account = client.account();
    let raw = match account
        .fetch_account_data_static::<IgnoredUserListEventContent>()
        .await
    {
        Ok(raw) => raw,
        Err(_) => account
            .account_data::<IgnoredUserListEventContent>()
            .await
            .ok()
            .flatten(),
    };
    let mut ids: Vec<String> = raw
        .and_then(|raw| raw.deserialize().ok())
        .map(|c| c.ignored_users.keys().map(|u| u.to_string()).collect())
        .unwrap_or_default();
    ids.sort();
    ids
}

/// The SDK edits its locally synced copy of the list, which loses a change that
/// was made a moment ago and not yet echoed by sync; edit the server's copy.
async fn edit(
    client: &Client,
    change: impl FnOnce(&mut IgnoredUserListEventContent),
) -> Result<(), IgnoreError> {
    let err = |e: &dyn std::fmt::Display| IgnoreError::Matrix(e.to_string());
    let account = client.account();
    let mut list = account
        .fetch_account_data_static::<IgnoredUserListEventContent>()
        .await
        .map_err(|e| err(&e))?
        .map(|raw| raw.deserialize())
        .transpose()
        .map_err(|e| err(&e))?
        .unwrap_or_default();
    change(&mut list);
    account.set_account_data(list).await.map_err(|e| err(&e))?;
    Ok(())
}

pub async fn block(client: &Client, mxid: &str) -> Result<(), IgnoreError> {
    let uid = parse(mxid)?;
    if client.user_id() == Some(uid) {
        return Err(IgnoreError::Matrix("you cannot block yourself".into()));
    }
    edit(client, |l| {
        l.ignored_users.insert(uid.to_owned(), IgnoredUser::new());
    })
    .await
}

pub async fn unblock(client: &Client, mxid: &str) -> Result<(), IgnoreError> {
    let uid = parse(mxid)?;
    edit(client, |l| {
        l.ignored_users.remove(uid);
    })
    .await
}
