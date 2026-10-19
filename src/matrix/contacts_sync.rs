//! Read/merge/write of the private contacts account-data event.
//!
//! Account data has no field-level merge, so every write is
//! fetch -> merge (per-contact LWW) -> write. A small race window remains
//! between fetch and write; a concurrent edit lost there is picked up
//! again at the next sync and re-merged.

use matrix_sdk::Account;
use matrix_sdk::ruma::events::GlobalAccountDataEventType;
use matrix_sdk::ruma::serde::Raw;
use thiserror::Error;

use crate::contacts::synchronization::{ACCOUNT_DATA_TYPE, ContactsDocument, ParseOutcome};

#[derive(Debug, Error)]
pub enum ContactsSyncError {
    #[error("could not sync contacts: {0}")]
    Matrix(String),
    #[error("contacts on the server use a newer format (v{0}); refusing to overwrite them")]
    NewerSchema(u32),
}

fn event_type() -> GlobalAccountDataEventType {
    GlobalAccountDataEventType::from(ACCOUNT_DATA_TYPE)
}

pub async fn fetch(
    account: &Account,
) -> Result<(ContactsDocument, ParseOutcome), ContactsSyncError> {
    let raw = account
        .fetch_account_data(event_type())
        .await
        .map_err(|e| ContactsSyncError::Matrix(e.to_string()))?;
    match raw {
        None => Ok((ContactsDocument::default(), ParseOutcome::Ok)),
        Some(raw) => {
            let v: serde_json::Value =
                serde_json::from_str(raw.json().get()).unwrap_or(serde_json::Value::Null);
            Ok(ContactsDocument::parse(&v))
        }
    }
}

/// Merge `local` with the server copy, write back if needed, return the merged result.
pub async fn sync(
    account: &Account,
    local: &ContactsDocument,
) -> Result<ContactsDocument, ContactsSyncError> {
    let (remote, outcome) = fetch(account).await?;
    if let ParseOutcome::NewerSchema(v) = outcome {
        return Err(ContactsSyncError::NewerSchema(v));
    }
    let mut merged = remote.clone();
    merged.merge(local);
    if merged != remote {
        let raw = Raw::new(&merged.to_value())
            .map_err(|e| ContactsSyncError::Matrix(e.to_string()))?
            .cast_unchecked();
        account
            .set_account_data_raw(event_type(), raw)
            .await
            .map_err(|e| ContactsSyncError::Matrix(e.to_string()))?;
    }
    Ok(merged)
}
