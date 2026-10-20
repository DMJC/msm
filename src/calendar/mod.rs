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

//! Calendar invitations carried in ordinary Matrix room messages.
//!
//! An invitation is an `m.room.message` (`m.text`) whose body is a readable
//! summary, so other clients just show text; msm also reads the structured
//! `com.matrixmessenger.calendar.v1` field. An RSVP is another text message
//! carrying the same field. An invitation only reaches the recipient's
//! calendar once they answer Tentative or Accepted.

use chrono::{Local, LocalResult, NaiveDate, NaiveTime, TimeZone};
use serde_json::{Value, json};

pub const KEY: &str = "com.matrixmessenger.calendar.v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub title: String,
    /// UTC milliseconds since the epoch.
    pub start_ms: u64,
    pub description: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Response {
    Tentative,
    Accepted,
    Declined,
}

impl Response {
    pub const ALL: [Response; 3] = [Response::Tentative, Response::Accepted, Response::Declined];

    pub fn key(self) -> &'static str {
        match self {
            Response::Tentative => "tentative",
            Response::Accepted => "accepted",
            Response::Declined => "declined",
        }
    }

    pub fn from_key(k: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|r| r.key() == k)
    }

    pub fn label(self) -> &'static str {
        match self {
            Response::Tentative => "Tentative",
            Response::Accepted => "Accept",
            Response::Declined => "Decline",
        }
    }

    /// Whether the event belongs on the calendar after this answer.
    pub fn on_calendar(self) -> bool {
        self != Response::Declined
    }
}

/// What a message's structured field says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    Invite(Invite),
    Rsvp {
        /// Event ID of the invitation being answered.
        event_id: String,
        response: Response,
    },
    /// The organiser changed the title, time or details of an invitation.
    Update {
        event_id: String,
        invite: Invite,
    },
    /// The organiser cancelled an invitation.
    Cancel {
        event_id: String,
    },
}

/// Local date/time text → UTC ms. `date` is `YYYY-MM-DD`, `time` is `HH:MM`.
pub fn parse_local(date: &str, time: &str) -> Option<u64> {
    let d = NaiveDate::parse_from_str(date.trim(), "%Y-%m-%d").ok()?;
    let t = NaiveTime::parse_from_str(time.trim(), "%H:%M").ok()?;
    match Local.from_local_datetime(&d.and_time(t)) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => {
            u64::try_from(dt.timestamp_millis()).ok()
        }
        LocalResult::None => None,
    }
}

/// UTC ms → local (`YYYY-MM-DD`, `HH:MM`), the inverse of [`parse_local`].
pub fn split_local(ms: u64) -> Option<(String, String)> {
    match Local.timestamp_millis_opt(ms as i64) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => Some((
            dt.format("%Y-%m-%d").to_string(),
            dt.format("%H:%M").to_string(),
        )),
        LocalResult::None => None,
    }
}

/// UTC ms → "Wed 21 Oct 2026, 14:00" in the viewer's time zone.
pub fn format_start(ms: u64) -> String {
    match Local.timestamp_millis_opt(ms as i64) {
        LocalResult::Single(dt) | LocalResult::Ambiguous(dt, _) => {
            dt.format("%a %d %b %Y, %H:%M").to_string()
        }
        LocalResult::None => "unknown time".into(),
    }
}

/// Message content for an invitation.
pub fn invite_content(inv: &Invite) -> Value {
    let mut body = format!(
        "📅 Invitation: {} — {}",
        inv.title,
        format_start(inv.start_ms)
    );
    if !inv.description.trim().is_empty() {
        body.push_str(&format!("\n{}", inv.description.trim()));
    }
    json!({
        "msgtype": "m.text",
        "body": body,
        KEY: {
            "kind": "invite",
            "title": inv.title,
            "start_ms": inv.start_ms,
            "description": inv.description,
        }
    })
}

/// Message content for an answer to the invitation `event_id`.
pub fn rsvp_content(title: &str, event_id: &str, response: Response) -> Value {
    let verb = match response {
        Response::Tentative => "tentatively accepted",
        Response::Accepted => "accepted",
        Response::Declined => "declined",
    };
    json!({
        "msgtype": "m.text",
        "body": format!("📅 I {verb}: {title}"),
        KEY: { "kind": "rsvp", "event_id": event_id, "response": response.key() }
    })
}

/// Message content changing invitation `event_id` (only honoured from its organiser).
pub fn update_content(event_id: &str, inv: &Invite) -> Value {
    json!({
        "msgtype": "m.text",
        "body": format!("📅 Updated: {} — {}", inv.title, format_start(inv.start_ms)),
        KEY: {
            "kind": "update",
            "event_id": event_id,
            "title": inv.title,
            "start_ms": inv.start_ms,
            "description": inv.description,
        }
    })
}

/// Message content cancelling invitation `event_id` (only honoured from its organiser).
pub fn cancel_content(title: &str, event_id: &str) -> Value {
    json!({
        "msgtype": "m.text",
        "body": format!("📅 Cancelled: {title}"),
        KEY: { "kind": "cancel", "event_id": event_id }
    })
}

fn parse_invite(c: &Value) -> Option<Invite> {
    let title = c.get("title")?.as_str()?.trim();
    if title.is_empty() {
        return None;
    }
    Some(Invite {
        title: title.chars().take(200).collect(),
        start_ms: c.get("start_ms")?.as_u64()?,
        description: c
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .chars()
            .take(2000)
            .collect(),
    })
}

/// Read the structured field out of a message's `content`.
pub fn parse(content: &Value) -> Option<Entry> {
    let c = content.get(KEY)?;
    match c.get("kind")?.as_str()? {
        "invite" => parse_invite(c).map(Entry::Invite),
        "update" => Some(Entry::Update {
            event_id: c.get("event_id")?.as_str()?.to_owned(),
            invite: parse_invite(c)?,
        }),
        "cancel" => Some(Entry::Cancel {
            event_id: c.get("event_id")?.as_str()?.to_owned(),
        }),
        "rsvp" => Some(Entry::Rsvp {
            event_id: c.get("event_id")?.as_str()?.to_owned(),
            response: Response::from_key(c.get("response")?.as_str()?)?,
        }),
        _ => None,
    }
}

/// How long before an event its reminder fires.
pub const REMINDER_LEAD_MS: u64 = 10 * 60 * 1000;
/// An event that began this recently still gets a reminder (the app was closed or asleep).
pub const REMINDER_GRACE_MS: u64 = 5 * 60 * 1000;

/// Whether an event starting at `start_ms` is due a reminder at `now_ms`.
pub fn reminder_due(start_ms: u64, now_ms: u64) -> bool {
    start_ms <= now_ms.saturating_add(REMINDER_LEAD_MS)
        && start_ms.saturating_add(REMINDER_GRACE_MS) > now_ms
}

/// "starting now", "in 7 min" or "5 min ago" for the reminder text.
pub fn starts_in(start_ms: u64, now_ms: u64) -> String {
    if start_ms >= now_ms {
        let mins = (start_ms - now_ms).div_ceil(60_000);
        if mins == 0 {
            "starting now".into()
        } else {
            format!("in {mins} min")
        }
    } else {
        let mins = (now_ms - start_ms) / 60_000;
        if mins == 0 {
            "starting now".into()
        } else {
            format!("started {mins} min ago")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invite() -> Invite {
        Invite {
            title: "Lunch".into(),
            start_ms: 1_800_000_000_000,
            description: "At the usual place".into(),
        }
    }

    #[test]
    fn invite_round_trips_and_has_a_readable_body() {
        let c = invite_content(&invite());
        assert!(c["body"].as_str().unwrap().contains("Lunch"));
        assert_eq!(c["msgtype"], "m.text");
        assert_eq!(parse(&c), Some(Entry::Invite(invite())));
    }

    #[test]
    fn rsvp_round_trips() {
        for r in Response::ALL {
            let c = rsvp_content("Lunch", "$abc", r);
            assert_eq!(
                parse(&c),
                Some(Entry::Rsvp {
                    event_id: "$abc".into(),
                    response: r
                })
            );
        }
    }

    #[test]
    fn update_and_cancel_round_trip() {
        let mut changed = invite();
        changed.start_ms += 3_600_000;
        assert_eq!(
            parse(&update_content("$abc", &changed)),
            Some(Entry::Update {
                event_id: "$abc".into(),
                invite: changed
            })
        );
        let c = cancel_content("Lunch", "$abc");
        assert!(c["body"].as_str().unwrap().contains("Cancelled"));
        assert_eq!(
            parse(&c),
            Some(Entry::Cancel {
                event_id: "$abc".into()
            })
        );
        assert_eq!(
            parse(&json!({KEY: {"kind":"update","event_id":"$a","title":"","start_ms":1}})),
            None
        );
    }

    #[test]
    fn malformed_or_foreign_content_is_ignored() {
        assert_eq!(parse(&json!({"msgtype":"m.text","body":"hi"})), None);
        assert_eq!(
            parse(&json!({KEY: {"kind":"invite","title":"  ","start_ms":1}})),
            None
        );
        assert_eq!(
            parse(&json!({KEY: {"kind":"invite","title":"x","start_ms":"soon"}})),
            None
        );
        assert_eq!(
            parse(&json!({KEY: {"kind":"rsvp","event_id":"$a","response":"maybe"}})),
            None
        );
        assert_eq!(parse(&json!({KEY: {"kind":"party"}})), None);
    }

    #[test]
    fn local_time_parsing_is_strict_and_round_trips_through_formatting() {
        assert!(parse_local("2026-13-01", "10:00").is_none());
        assert!(parse_local("2026-10-21", "25:00").is_none());
        assert!(parse_local("tomorrow", "10:00").is_none());
        let ms = parse_local("2026-10-21", "14:30").unwrap();
        assert_eq!(split_local(ms), Some(("2026-10-21".into(), "14:30".into())));
        assert!(format_start(ms).ends_with("14:30"));
    }

    #[test]
    fn reminders_fire_in_the_lead_window_and_shortly_after_the_start() {
        let now = 1_000_000_000u64;
        let min = 60_000;
        assert!(!reminder_due(now + 11 * min, now), "too early");
        assert!(reminder_due(now + 10 * min, now));
        assert!(reminder_due(now, now));
        assert!(reminder_due(now - 4 * min, now), "just started");
        assert!(!reminder_due(now - 6 * min, now), "long past");
        assert_eq!(starts_in(now + 7 * min - 1, now), "in 7 min");
        assert_eq!(starts_in(now, now), "starting now");
        assert_eq!(starts_in(now - 3 * min, now), "started 3 min ago");
    }

    #[test]
    fn only_tentative_and_accepted_reach_the_calendar() {
        assert!(Response::Tentative.on_calendar());
        assert!(Response::Accepted.on_calendar());
        assert!(!Response::Declined.on_calendar());
    }
}
