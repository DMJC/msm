//! Centralised name resolution. All UI code must go through [`NameResolver`].
//!
//! Priority: local nickname > room-specific display name > global profile
//! display name > Matrix user ID.

use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub struct NameResolver {
    nicknames: HashMap<String, String>,
    profile_names: HashMap<String, String>,
    /// (room_id, mxid) -> room-specific display name
    room_names: HashMap<(String, String), String>,
}

fn clean(s: &str) -> Option<&str> {
    let t = s.trim();
    (!t.is_empty()).then_some(t)
}

impl NameResolver {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set or clear (None / blank) a private nickname.
    pub fn set_nickname(&mut self, mxid: &str, nickname: Option<&str>) {
        match nickname.and_then(clean) {
            Some(n) => {
                self.nicknames.insert(mxid.to_owned(), n.to_owned());
            }
            None => {
                self.nicknames.remove(mxid);
            }
        }
    }

    pub fn set_profile_name(&mut self, mxid: &str, name: Option<&str>) {
        match name.and_then(clean) {
            Some(n) => {
                self.profile_names.insert(mxid.to_owned(), n.to_owned());
            }
            None => {
                self.profile_names.remove(mxid);
            }
        }
    }

    pub fn set_room_name(&mut self, room_id: &str, mxid: &str, name: Option<&str>) {
        let key = (room_id.to_owned(), mxid.to_owned());
        match name.and_then(clean) {
            Some(n) => {
                self.room_names.insert(key, n.to_owned());
            }
            None => {
                self.room_names.remove(&key);
            }
        }
    }

    pub fn nickname(&self, mxid: &str) -> Option<&str> {
        self.nicknames.get(mxid).map(String::as_str)
    }

    pub fn resolve(&self, mxid: &str, room_id: Option<&str>) -> String {
        if let Some(n) = self.nicknames.get(mxid) {
            return n.clone();
        }
        if let Some(room) = room_id {
            if let Some(n) = self.room_names.get(&(room.to_owned(), mxid.to_owned())) {
                return n.clone();
            }
        }
        if let Some(n) = self.profile_names.get(mxid) {
            return n.clone();
        }
        mxid.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "@dmjc:example.com";

    #[test]
    fn priority_order() {
        let mut r = NameResolver::new();
        assert_eq!(r.resolve(U, None), U);
        r.set_profile_name(U, Some("Darkmage"));
        assert_eq!(r.resolve(U, None), "Darkmage");
        r.set_room_name("!r:x", U, Some("Dark (room)"));
        assert_eq!(r.resolve(U, Some("!r:x")), "Dark (room)");
        assert_eq!(r.resolve(U, Some("!other:x")), "Darkmage");
        r.set_nickname(U, Some("James Carthew"));
        assert_eq!(r.resolve(U, Some("!r:x")), "James Carthew");
        assert_eq!(r.resolve(U, None), "James Carthew");
    }

    #[test]
    fn reset_nickname_falls_back_immediately() {
        let mut r = NameResolver::new();
        r.set_profile_name(U, Some("Darkmage"));
        r.set_nickname(U, Some("DMJC"));
        r.set_nickname(U, None);
        assert_eq!(r.resolve(U, None), "Darkmage");
        r.set_nickname(U, Some("   "));
        assert_eq!(r.resolve(U, None), "Darkmage");
    }

    #[test]
    fn missing_and_blank_display_names() {
        let mut r = NameResolver::new();
        r.set_profile_name(U, Some("  "));
        assert_eq!(r.resolve(U, None), U);
    }

    #[test]
    fn duplicate_display_names_stay_distinct_by_mxid() {
        let mut r = NameResolver::new();
        r.set_profile_name("@a:x", Some("Sam"));
        r.set_profile_name("@b:x", Some("Sam"));
        r.set_nickname("@b:x", Some("Sam B"));
        assert_eq!(r.resolve("@a:x", None), "Sam");
        assert_eq!(r.resolve("@b:x", None), "Sam B");
    }

    #[test]
    fn unicode_nicknames_preserved_and_trimmed() {
        let mut r = NameResolver::new();
        r.set_nickname(U, Some("  日本語 🦀 "));
        assert_eq!(r.resolve(U, None), "日本語 🦀");
    }

    #[test]
    fn nicknames_are_per_resolver() {
        let mut alice = NameResolver::new();
        let mut bob = NameResolver::new();
        alice.set_nickname(U, Some("James Carthew"));
        bob.set_nickname(U, Some("DMJC"));
        assert_eq!(alice.resolve(U, None), "James Carthew");
        assert_eq!(bob.resolve(U, None), "DMJC");
    }
}
