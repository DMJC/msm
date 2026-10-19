//! MSN-style status <-> Matrix presence mapping.
//!
//! Matrix presence has only three values (online / unavailable / offline), so:
//!
//! | UI status | wire presence | wire `status_msg`                    |
//! |-----------|---------------|--------------------------------------|
//! | Online    | online        | user message (if any)                |
//! | Away      | unavailable   | user message (if any)                |
//! | Busy      | unavailable   | `Busy` or `Busy - <user message>`    |
//! | Offline   | offline       | user message (if any)                |
//!
//! Other Matrix clients see Busy as "unavailable" with a "Busy" message.
//! When reading a contact's presence we recognise that prefix as Busy.
//! A contact with no presence information is `Unknown`, never `Offline`:
//! servers may disable presence or hide it from us.

use matrix_sdk::Client;
use matrix_sdk::ruma::api::client::presence::set_presence;
use matrix_sdk::ruma::presence::PresenceState;

const BUSY: &str = "Busy";

/// Statuses the user can pick for themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UiStatus {
    #[default]
    Online,
    Away,
    Busy,
    Offline,
}

/// What we know about someone else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactPresence {
    Online,
    Away,
    Busy,
    Offline,
    Unknown,
}

impl UiStatus {
    pub const ALL: [UiStatus; 4] = [
        UiStatus::Online,
        UiStatus::Away,
        UiStatus::Busy,
        UiStatus::Offline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            UiStatus::Online => "Online",
            UiStatus::Away => "Away",
            UiStatus::Busy => "Busy",
            UiStatus::Offline => "Appear offline",
        }
    }

    /// Stable key for persistence.
    pub fn key(self) -> &'static str {
        match self {
            UiStatus::Online => "online",
            UiStatus::Away => "away",
            UiStatus::Busy => "busy",
            UiStatus::Offline => "offline",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.key() == k)
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|s| *s == self).unwrap_or(0)
    }

    /// Wire presence and status message to send to the homeserver.
    pub fn to_wire(self, message: &str) -> (PresenceState, Option<String>) {
        let msg = message.trim();
        let user = (!msg.is_empty()).then(|| msg.to_owned());
        match self {
            UiStatus::Online => (PresenceState::Online, user),
            UiStatus::Away => (PresenceState::Unavailable, user),
            UiStatus::Busy => (
                PresenceState::Unavailable,
                Some(match user {
                    Some(m) => format!("{BUSY} - {m}"),
                    None => BUSY.to_owned(),
                }),
            ),
            UiStatus::Offline => (PresenceState::Offline, user),
        }
    }
}

/// Interpret a presence update. Returns the state and the user-visible message
/// (without our Busy marker).
pub fn from_wire(
    presence: &PresenceState,
    status_msg: Option<&str>,
) -> (ContactPresence, Option<String>) {
    let msg = status_msg.map(str::trim).filter(|s| !s.is_empty());
    match presence {
        PresenceState::Online => (ContactPresence::Online, msg.map(str::to_owned)),
        PresenceState::Offline => (ContactPresence::Offline, msg.map(str::to_owned)),
        PresenceState::Unavailable => match msg {
            Some(m) if m == BUSY => (ContactPresence::Busy, None),
            Some(m) if m.strip_prefix(BUSY).is_some_and(|r| r.starts_with(" - ")) => {
                let rest = m[BUSY.len() + 3..].trim();
                (
                    ContactPresence::Busy,
                    (!rest.is_empty()).then(|| rest.to_owned()),
                )
            }
            other => (ContactPresence::Away, other.map(str::to_owned)),
        },
        _ => (ContactPresence::Unknown, msg.map(str::to_owned)),
    }
}

/// The user's chosen status plus automatic-Away bookkeeping.
///
/// Rules: only an *Online* user is moved to Away when idle (someone who chose
/// Busy, Away or Appear offline meant it); any manual choice cancels automatic
/// Away; activity restores Online only if Away was automatic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AutoAway {
    manual: UiStatus,
    auto: bool,
}

impl AutoAway {
    pub fn new(manual: UiStatus) -> Self {
        Self {
            manual,
            auto: false,
        }
    }

    /// What should be published and displayed.
    pub fn effective(&self) -> UiStatus {
        if self.auto {
            UiStatus::Away
        } else {
            self.manual
        }
    }

    /// The user's own choice (what is persisted).
    pub fn manual(&self) -> UiStatus {
        self.manual
    }

    pub fn is_auto(&self) -> bool {
        self.auto
    }

    pub fn set_manual(&mut self, s: UiStatus) {
        self.manual = s;
        self.auto = false;
    }

    /// The user went idle. Returns whether the effective status changed.
    pub fn idle(&mut self) -> bool {
        if self.manual == UiStatus::Online && !self.auto {
            self.auto = true;
            return true;
        }
        false
    }

    /// The user is active again. Returns whether the effective status changed.
    pub fn resume(&mut self) -> bool {
        std::mem::take(&mut self.auto)
    }
}

/// The status (and message) we want the homeserver to show for us.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Desired {
    pub status: UiStatus,
    pub message: String,
}

impl Desired {
    pub fn new(status: UiStatus, message: &str) -> Self {
        Self {
            status,
            message: message.to_owned(),
        }
    }

    pub fn wire(&self) -> PresenceState {
        self.status.to_wire(&self.message).0
    }
}

/// Publish our own status to the homeserver. Normally called by the sync loop,
/// which also tags every `/sync` with the wire presence (otherwise `/sync`
/// would mark us online again).
pub async fn publish(client: &Client, desired: &Desired) -> Result<(), String> {
    let uid = client.user_id().ok_or("not logged in")?.to_owned();
    let (wire, msg) = desired.status.to_wire(&desired.message);
    let mut req = set_presence::v3::Request::new(uid, wire);
    req.status_msg = msg;
    client.send(req).await.map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_ui_status_with_and_without_message() {
        for s in UiStatus::ALL {
            for m in ["", "  ", "at lunch 🍜", "Busy"] {
                let (wire, msg) = s.to_wire(m);
                let (back, text) = from_wire(&wire, msg.as_deref());
                let expected = match s {
                    UiStatus::Online => ContactPresence::Online,
                    UiStatus::Away => ContactPresence::Away,
                    UiStatus::Busy => ContactPresence::Busy,
                    UiStatus::Offline => ContactPresence::Offline,
                };
                // A user-typed "Busy" while Away is indistinguishable on the wire by design.
                if s == UiStatus::Away && m == "Busy" {
                    assert_eq!(back, ContactPresence::Busy);
                    continue;
                }
                assert_eq!(back, expected, "{s:?} {m:?}");
                let want = (!m.trim().is_empty()).then(|| m.trim().to_owned());
                assert_eq!(text, want, "{s:?} {m:?}");
            }
        }
    }

    #[test]
    fn busy_wire_format_is_readable_by_other_clients() {
        assert_eq!(
            UiStatus::Busy.to_wire(""),
            (PresenceState::Unavailable, Some("Busy".into()))
        );
        assert_eq!(
            UiStatus::Busy.to_wire("in a meeting"),
            (
                PresenceState::Unavailable,
                Some("Busy - in a meeting".into())
            )
        );
    }

    #[test]
    fn plain_unavailable_is_away_and_busyish_text_is_not_busy() {
        assert_eq!(
            from_wire(&PresenceState::Unavailable, None).0,
            ContactPresence::Away
        );
        assert_eq!(
            from_wire(&PresenceState::Unavailable, Some("Busybody")).0,
            ContactPresence::Away
        );
    }

    #[test]
    fn unknown_wire_state_is_unknown_not_offline() {
        let custom = PresenceState::from("something-new");
        assert_eq!(from_wire(&custom, None).0, ContactPresence::Unknown);
    }

    #[test]
    fn keys_round_trip() {
        for s in UiStatus::ALL {
            assert_eq!(UiStatus::from_key(s.key()), Some(s));
            assert_eq!(UiStatus::ALL[s.index()], s);
        }
        assert_eq!(UiStatus::from_key("nope"), None);
    }

    #[test]
    fn auto_away_only_affects_online_users() {
        for manual in [UiStatus::Away, UiStatus::Busy, UiStatus::Offline] {
            let mut a = AutoAway::new(manual);
            assert!(!a.idle(), "{manual:?} must not change when idle");
            assert_eq!(a.effective(), manual);
            assert!(!a.resume());
            assert_eq!(a.effective(), manual);
        }
        let mut a = AutoAway::new(UiStatus::Online);
        assert!(a.idle());
        assert_eq!(a.effective(), UiStatus::Away);
        assert_eq!(a.manual(), UiStatus::Online);
        assert!(!a.idle(), "second idle is a no-op");
        assert!(a.resume());
        assert_eq!(a.effective(), UiStatus::Online);
        assert!(!a.resume(), "resume without auto-away is a no-op");
    }

    #[test]
    fn manual_choice_cancels_auto_away() {
        let mut a = AutoAway::new(UiStatus::Online);
        a.idle();
        a.set_manual(UiStatus::Busy);
        assert!(!a.is_auto());
        assert_eq!(a.effective(), UiStatus::Busy);
        // Activity afterwards must not override the manual choice.
        assert!(!a.resume());
        assert_eq!(a.effective(), UiStatus::Busy);
        // Choosing Online while auto-away returns to plain Online.
        let mut b = AutoAway::new(UiStatus::Online);
        b.idle();
        b.set_manual(UiStatus::Online);
        assert_eq!(b.effective(), UiStatus::Online);
        assert!(!b.is_auto());
    }
}
