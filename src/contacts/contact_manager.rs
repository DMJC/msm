//! Contact operations over a [`ContactsDocument`], keeping the shared
//! [`NameResolver`] in sync so every UI component sees nickname changes at once.

use thiserror::Error;

use super::contact::{Contact, is_valid_mxid};
use super::nickname_resolver::NameResolver;
use super::synchronization::ContactsDocument;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ContactError {
    #[error("invalid Matrix ID: {0}")]
    InvalidMxid(String),
    #[error("contact not found: {0}")]
    NotFound(String),
    #[error("group already exists: {0}")]
    GroupExists(String),
    #[error("group not found: {0}")]
    GroupNotFound(String),
    #[error("group name must not be empty")]
    EmptyGroupName,
}

#[derive(Debug, Default)]
pub struct ContactManager {
    doc: ContactsDocument,
    resolver: NameResolver,
}

impl ContactManager {
    pub fn new(doc: ContactsDocument) -> Self {
        let mut m = Self {
            doc,
            resolver: NameResolver::new(),
        };
        m.rebuild_resolver();
        m
    }

    pub fn document(&self) -> &ContactsDocument {
        &self.doc
    }

    pub fn resolver(&self) -> &NameResolver {
        &self.resolver
    }

    pub fn resolver_mut(&mut self) -> &mut NameResolver {
        &mut self.resolver
    }

    /// Replace state after a remote merge; nicknames update immediately.
    pub fn apply_remote(&mut self, remote: &ContactsDocument) {
        self.doc.merge(remote);
        self.rebuild_resolver();
    }

    fn rebuild_resolver(&mut self) {
        let keys: Vec<String> = self.doc.contacts.keys().cloned().collect();
        for k in keys {
            let nick = self
                .doc
                .contacts
                .get(&k)
                .filter(|c| !c.deleted)
                .and_then(|c| c.nickname.clone());
            self.resolver.set_nickname(&k, nick.as_deref());
        }
    }

    pub fn get(&self, mxid: &str) -> Option<&Contact> {
        self.doc.contacts.get(mxid).filter(|c| !c.deleted)
    }

    pub fn list(&self) -> Vec<&Contact> {
        self.doc.live().collect()
    }

    pub fn search(&self, query: &str) -> Vec<&Contact> {
        let q = query.trim().to_lowercase();
        self.doc
            .live()
            .filter(|c| {
                q.is_empty()
                    || c.mxid.to_lowercase().contains(&q)
                    || self
                        .resolver
                        .resolve(&c.mxid, None)
                        .to_lowercase()
                        .contains(&q)
            })
            .collect()
    }

    pub fn add(&mut self, mxid: &str, now: u64) -> Result<(), ContactError> {
        let mxid = mxid.trim();
        if !is_valid_mxid(mxid) {
            return Err(ContactError::InvalidMxid(mxid.to_owned()));
        }
        let mut c = Contact::new(mxid, now);
        if let Some(old) = self.doc.contacts.get(mxid) {
            // Re-adding a tombstoned or existing contact: keep prior settings, bump the clock.
            if !old.deleted {
                return Ok(());
            }
            c.updated_at = now.max(old.updated_at + 1);
        }
        self.doc.contacts.insert(mxid.to_owned(), c);
        Ok(())
    }

    pub fn remove(&mut self, mxid: &str, now: u64) -> Result<(), ContactError> {
        let c = self.live_mut(mxid)?;
        c.deleted = true;
        c.nickname = None;
        c.updated_at = bump(c.updated_at, now);
        self.resolver.set_nickname(mxid, None);
        Ok(())
    }

    /// `None` or blank resets to the public display name.
    pub fn set_nickname(
        &mut self,
        mxid: &str,
        nick: Option<&str>,
        now: u64,
    ) -> Result<(), ContactError> {
        let nick = nick.map(str::trim).filter(|s| !s.is_empty());
        let c = self.live_mut(mxid)?;
        c.nickname = nick.map(str::to_owned);
        c.updated_at = bump(c.updated_at, now);
        self.resolver.set_nickname(mxid, nick);
        Ok(())
    }

    pub fn set_favorite(&mut self, mxid: &str, fav: bool, now: u64) -> Result<(), ContactError> {
        let c = self.live_mut(mxid)?;
        c.favorite = fav;
        c.updated_at = bump(c.updated_at, now);
        Ok(())
    }

    pub fn move_to_group(&mut self, mxid: &str, group: &str, now: u64) -> Result<(), ContactError> {
        if !self.doc.groups.iter().any(|g| g == group) {
            return Err(ContactError::GroupNotFound(group.to_owned()));
        }
        let c = self.live_mut(mxid)?;
        c.group = group.to_owned();
        c.updated_at = bump(c.updated_at, now);
        Ok(())
    }

    pub fn create_group(&mut self, name: &str) -> Result<(), ContactError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ContactError::EmptyGroupName);
        }
        if self.doc.groups.iter().any(|g| g == name) {
            return Err(ContactError::GroupExists(name.to_owned()));
        }
        self.doc.groups.push(name.to_owned());
        Ok(())
    }

    pub fn rename_group(&mut self, old: &str, new: &str, now: u64) -> Result<(), ContactError> {
        let new = new.trim();
        if new.is_empty() {
            return Err(ContactError::EmptyGroupName);
        }
        if old != new && self.doc.groups.iter().any(|g| g == new) {
            return Err(ContactError::GroupExists(new.to_owned()));
        }
        let idx = self.group_idx(old)?;
        self.doc.groups[idx] = new.to_owned();
        for c in self.doc.contacts.values_mut().filter(|c| c.group == old) {
            c.group = new.to_owned();
            c.updated_at = bump(c.updated_at, now);
        }
        Ok(())
    }

    /// Deleting a group moves its contacts to `fallback` (must exist).
    pub fn delete_group(
        &mut self,
        name: &str,
        fallback: &str,
        now: u64,
    ) -> Result<(), ContactError> {
        let idx = self.group_idx(name)?;
        if name == fallback || !self.doc.groups.iter().any(|g| g == fallback) {
            return Err(ContactError::GroupNotFound(fallback.to_owned()));
        }
        self.doc.groups.remove(idx);
        for c in self.doc.contacts.values_mut().filter(|c| c.group == name) {
            c.group = fallback.to_owned();
            c.updated_at = bump(c.updated_at, now);
        }
        Ok(())
    }

    pub fn move_group(&mut self, name: &str, new_index: usize) -> Result<(), ContactError> {
        let idx = self.group_idx(name)?;
        let g = self.doc.groups.remove(idx);
        self.doc
            .groups
            .insert(new_index.min(self.doc.groups.len()), g);
        Ok(())
    }

    fn group_idx(&self, name: &str) -> Result<usize, ContactError> {
        self.doc
            .groups
            .iter()
            .position(|g| g == name)
            .ok_or_else(|| ContactError::GroupNotFound(name.to_owned()))
    }

    fn live_mut(&mut self, mxid: &str) -> Result<&mut Contact, ContactError> {
        self.doc
            .contacts
            .get_mut(mxid)
            .filter(|c| !c.deleted)
            .ok_or_else(|| ContactError::NotFound(mxid.to_owned()))
    }
}

/// Strictly increasing per-record clock, robust to skew.
fn bump(prev: u64, now: u64) -> u64 {
    now.max(prev + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    const U: &str = "@dmjc:example.com";

    fn mgr() -> ContactManager {
        let mut m = ContactManager::default();
        m.add(U, 1).unwrap();
        m
    }

    #[test]
    fn add_validates_and_is_idempotent() {
        let mut m = ContactManager::default();
        assert!(matches!(
            m.add("dmjc", 1),
            Err(ContactError::InvalidMxid(_))
        ));
        m.add(U, 1).unwrap();
        m.set_nickname(U, Some("J"), 2).unwrap();
        m.add(U, 3).unwrap();
        assert_eq!(m.get(U).unwrap().nickname.as_deref(), Some("J"));
    }

    #[test]
    fn nickname_updates_resolver_and_resets() {
        let mut m = mgr();
        m.resolver_mut().set_profile_name(U, Some("Darkmage"));
        m.set_nickname(U, Some("James Carthew"), 2).unwrap();
        assert_eq!(m.resolver().resolve(U, None), "James Carthew");
        m.set_nickname(U, None, 3).unwrap();
        assert_eq!(m.resolver().resolve(U, None), "Darkmage");
    }

    #[test]
    fn remove_and_readd() {
        let mut m = mgr();
        m.remove(U, 2).unwrap();
        assert!(m.get(U).is_none());
        assert_eq!(m.remove(U, 3), Err(ContactError::NotFound(U.into())));
        m.add(U, 1).unwrap(); // stale clock still wins over tombstone
        assert!(m.get(U).is_some());
    }

    #[test]
    fn groups_lifecycle() {
        let mut m = mgr();
        m.create_group("Gamers").unwrap();
        assert_eq!(
            m.create_group("Gamers"),
            Err(ContactError::GroupExists("Gamers".into()))
        );
        m.move_to_group(U, "Gamers", 2).unwrap();
        m.rename_group("Gamers", "Mates", 3).unwrap();
        assert_eq!(m.get(U).unwrap().group, "Mates");
        m.move_group("Mates", 0).unwrap();
        assert_eq!(m.document().groups[0], "Mates");
        m.delete_group("Mates", "Other", 4).unwrap();
        assert_eq!(m.get(U).unwrap().group, "Other");
        assert!(m.move_to_group(U, "Mates", 5).is_err());
    }

    #[test]
    fn search_matches_nickname_and_mxid() {
        let mut m = mgr();
        m.set_nickname(U, Some("James Carthew"), 2).unwrap();
        assert_eq!(m.search("carthew").len(), 1);
        assert_eq!(m.search("dmjc").len(), 1);
        assert_eq!(m.search("zzz").len(), 0);
    }

    #[test]
    fn remote_merge_updates_names_immediately() {
        let mut a = mgr();
        let mut b = ContactManager::new(a.document().clone());
        b.set_nickname(U, Some("DMJC"), 10).unwrap();
        a.apply_remote(b.document());
        assert_eq!(a.resolver().resolve(U, None), "DMJC");
        let mut gone = ContactManager::new(a.document().clone());
        gone.remove(U, 20).unwrap();
        a.apply_remote(gone.document());
        assert_eq!(a.resolver().resolve(U, None), U);
    }
}
