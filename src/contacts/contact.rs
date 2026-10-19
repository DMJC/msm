//! Contact data model. Contacts are keyed by Matrix user ID, never by display name.

use serde::{Deserialize, Serialize};

pub const DEFAULT_GROUPS: [&str; 4] = ["Friends", "Family", "Work", "Other"];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub mxid: String,
    /// Private nickname; never written to the target's Matrix profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    #[serde(default = "default_group")]
    pub group: String,
    #[serde(default)]
    pub favorite: bool,
    /// Milliseconds since epoch of the last local modification; drives merging.
    #[serde(default)]
    pub updated_at: u64,
    /// Tombstone so deletions survive merges with stale devices.
    #[serde(default)]
    pub deleted: bool,
}

fn default_group() -> String {
    "Other".to_owned()
}

impl Contact {
    pub fn new(mxid: impl Into<String>, now: u64) -> Self {
        Self {
            mxid: mxid.into(),
            nickname: None,
            group: default_group(),
            favorite: false,
            updated_at: now,
            deleted: false,
        }
    }
}

/// Basic Matrix user ID validation: `@localpart:server`.
pub fn is_valid_mxid(s: &str) -> bool {
    let Some(rest) = s.strip_prefix('@') else {
        return false;
    };
    match rest.split_once(':') {
        Some((local, server)) => {
            !local.is_empty()
                && !server.is_empty()
                && !s.chars().any(|c| c.is_whitespace() || c.is_control())
        }
        None => false,
    }
}
