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

//! Sync loop running on the Tokio runtime. Results are sent over an
//! `async_channel` to the GTK main thread; no GTK types are touched here.

use std::time::Duration;

use matrix_sdk::Client;
use std::sync::Arc;

use matrix_sdk::config::SyncSettings;
use tokio::sync::watch;

use super::presence::{self, Desired};

#[derive(Debug, Clone)]
pub enum SyncEvent {
    Connected,
    /// A sync response was processed (room state / unread counts may have changed).
    Synced,
    /// Publishing our status failed (servers may disable or restrict presence).
    PresenceFailed(String),
    /// Transient failure; the loop keeps retrying with backoff.
    Disconnected(String),
    /// Server rejected our token; the user must log in again.
    SessionExpired,
}

/// Exponential backoff, capped. `attempt` starts at 0.
pub fn backoff(attempt: u32) -> Duration {
    Duration::from_secs((1u64 << attempt.min(6)).min(60))
}

/// Runs until the task is aborted. Never spins: every failure waits.
///
/// Also owns publishing our status. `/sync` marks us online unless told
/// otherwise, so every request carries the wanted wire presence. When the wanted
/// status changes, the in-flight long-poll is dropped, one quick (zero-timeout)
/// sync runs with the new presence, and only then is the status PUT (in a
/// background task, since some servers take seconds to answer). Publishing
/// earlier races with the server finishing the abandoned request, which could
/// flip us back to online.
pub async fn run(
    client: Client,
    tx: async_channel::Sender<SyncEvent>,
    mut desired: watch::Receiver<Desired>,
) {
    let mut attempt = 0u32;
    let mut first = true;
    let mut published: Option<Desired> = None;
    // Presence PUTs can be slow on some servers; they run off the sync loop, one at a time, in order.
    let publish_lock = Arc::new(tokio::sync::Mutex::new(()));
    let mut settings = SyncSettings::default().timeout(Duration::from_secs(30));
    loop {
        let want = desired.borrow_and_update().clone();
        let needs_publish = published.as_ref() != Some(&want);
        let mut request = settings.clone().set_presence(want.wire());
        if needs_publish {
            request = request.timeout(Duration::ZERO);
        }
        let changed = async {
            if desired.changed().await.is_err() {
                // Sender gone: keep syncing with the last status, don't spin.
                std::future::pending::<()>().await;
            }
        };
        let result = tokio::select! {
            r = client.sync_once(request) => r,
            _ = changed => continue,
        };
        match result {
            Ok(resp) => {
                if attempt > 0 || first {
                    let _ = tx.send(SyncEvent::Connected).await;
                }
                attempt = 0;
                first = false;
                settings = settings.token(resp.next_batch);
                if needs_publish {
                    // Recorded even on failure so a server without presence is not retried every sync.
                    published = Some(want.clone());
                    let (client, tx, lock, latest) = (
                        client.clone(),
                        tx.clone(),
                        publish_lock.clone(),
                        desired.clone(),
                    );
                    tokio::spawn(async move {
                        let _turn = lock.lock().await;
                        if *latest.borrow() != want {
                            return; // superseded; a newer status is queued behind us
                        }
                        if let Err(e) = presence::publish(&client, &want).await {
                            let _ = tx.send(SyncEvent::PresenceFailed(e)).await;
                        }
                    });
                }
                let _ = tx.send(SyncEvent::Synced).await;
            }
            Err(e) => {
                if e.client_api_error_kind().is_some_and(|k| {
                    matches!(
                        k,
                        matrix_sdk::ruma::api::error::ErrorKind::UnknownToken { .. }
                    )
                }) {
                    let _ = tx.send(SyncEvent::SessionExpired).await;
                    return;
                }
                let _ = tx.send(SyncEvent::Disconnected(e.to_string())).await;
                tokio::time::sleep(backoff(attempt)).await;
                attempt += 1;
            }
        }
        if tx.is_closed() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_and_caps() {
        assert_eq!(backoff(0), Duration::from_secs(1));
        assert_eq!(backoff(3), Duration::from_secs(8));
        assert_eq!(backoff(6), Duration::from_secs(60));
        assert_eq!(backoff(50), Duration::from_secs(60));
    }
}
