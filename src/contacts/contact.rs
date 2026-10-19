//! Contact data model. Contacts are keyed by Matrix user ID, never by display name.

use serde::{Deserialize, Serialize};

pub const DEFAULT_GROUPS: [&str; 4] = ["Friends", "Family", "Work", "Other"];

/// Application-level friendship, separate from Matrix room invitations.
/// `Friends` is only reached with a verifiable acknowledgement from the other
/// side (their accept for our request, or our accept of their request).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Friendship {
    /// Plain local contact; no friendship protocol involved.
    #[default]
    None,
    /// We asked; no answer yet.
    Outgoing,
    /// They asked; we have not answered.
    Incoming,
    Friends,
}

impl Friendship {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Outgoing => "outgoing",
            Self::Incoming => "incoming",
            Self::Friends => "friends",
        }
    }

    /// Unknown values degrade to `None`: never invent a friendship.
    pub fn parse(s: &str) -> Self {
        match s {
            "outgoing" => Self::Outgoing,
            "incoming" => Self::Incoming,
            "friends" => Self::Friends,
            _ => Self::None,
        }
    }
}

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
    #[serde(default, skip_serializing_if = "is_none")]
    pub friendship: Friendship,
    /// Milliseconds since epoch of the last local modification; drives merging.
    #[serde(default)]
    pub updated_at: u64,
    /// Tombstone so deletions survive merges with stale devices.
    #[serde(default)]
    pub deleted: bool,
}

fn is_none(f: &Friendship) -> bool {
    *f == Friendship::None
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
            friendship: Friendship::None,
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
