//! Sync loop running on the Tokio runtime. Results are sent over an
//! `async_channel` to the GTK main thread; no GTK types are touched here.

use std::time::Duration;

use matrix_sdk::Client;
use matrix_sdk::config::SyncSettings;

#[derive(Debug, Clone)]
pub enum SyncEvent {
    Connected,
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
pub async fn run(client: Client, tx: async_channel::Sender<SyncEvent>) {
    let mut attempt = 0u32;
    let mut first = true;
    let mut settings = SyncSettings::default().timeout(Duration::from_secs(30));
    loop {
        match client.sync_once(settings.clone()).await {
            Ok(resp) => {
                if attempt > 0 || first {
                    let _ = tx.send(SyncEvent::Connected).await;
                }
                attempt = 0;
                first = false;
                settings = settings.token(resp.next_batch);
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
