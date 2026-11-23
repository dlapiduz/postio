//! Stable UIDs for a maildir, written beside it as `.postio-uids` (#1278): `uidvalidity N`, then
//! `uid name` per line.
//! Keyed on the name without its flags, since marking read renames the file; numbers are never
//! reused.
//! Derived numbers would renumber the mailbox on any delivery or delete.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const FILE: &str = ".postio-uids";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UidList {
    /// Minted once from the clock. If the file is lost the new value makes the store treat every
    /// message as new, the honest answer.
    validity: u32,
    uids: BTreeMap<String, u32>,
    next: u32,
}

impl UidList {
    pub fn new(validity: u32) -> Self {
        Self {
            validity,
            uids: BTreeMap::new(),
            next: 1,
        }
    }

    /// Unreadable is empty, not an error: the recovery (fresh validity, re-read everything) matches
    /// a server's `UIDVALIDITY` change, and failing to open would make a corrupt sidecar into
    /// unreachable mail.
    pub fn load(folder: &Path, now: u32) -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path(folder)) else {
            return Self::new(now);
        };
        Self::parse(&text).unwrap_or_else(|| Self::new(now))
    }

    pub fn parse(text: &str) -> Option<Self> {
        let mut lines = text.lines();
        let validity = lines
            .next()?
            .strip_prefix("uidvalidity ")?
            .trim()
            .parse()
            .ok()?;
        let mut list = Self::new(validity);
        for line in lines {
            let Some((uid, name)) = line.split_once(' ') else {
                continue;
            };
            let Ok(uid) = uid.parse::<u32>() else {
                continue;
            };
            // A line naming nothing is skipped: renumbering one message beats renumbering the
            // folder.
            if name.is_empty() {
                continue;
            }
            list.uids.insert(name.to_owned(), uid);
            list.next = list.next.max(uid + 1);
        }
        Some(list)
    }

    pub fn render(&self) -> String {
        let mut out = format!("uidvalidity {}\n", self.validity);
        let mut entries: Vec<(&String, &u32)> = self.uids.iter().collect();
        entries.sort_by_key(|(_, uid)| **uid);
        for (name, uid) in entries {
            out.push_str(&format!("{uid} {name}\n"));
        }
        out
    }

    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        std::fs::write(Self::path(folder), self.render())
    }

    pub fn validity(&self) -> u32 {
        self.validity
    }

    pub fn next(&self) -> u32 {
        self.next
    }

    pub fn uid_for(&mut self, name: &str) -> u32 {
        let key = strip_flags(name);
        if let Some(uid) = self.uids.get(&key) {
            return *uid;
        }
        let uid = self.next;
        self.next += 1;
        self.uids.insert(key, uid);
        uid
    }

    pub fn known(&self, name: &str) -> Option<u32> {
        self.uids.get(&strip_flags(name)).copied()
    }

    /// Numbers stay spent (`next` never rewinds) so a later message cannot inherit the identity of
    /// one already read.
    pub fn retain(&mut self, present: &[String]) {
        let keys: std::collections::BTreeSet<String> =
            present.iter().map(|name| strip_flags(name)).collect();
        self.uids.retain(|name, _| keys.contains(name));
    }

    pub fn len(&self) -> usize {
        self.uids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.uids.is_empty()
    }

    fn path(folder: &Path) -> PathBuf {
        folder.join(FILE)
    }
}

fn strip_flags(name: &str) -> String {
    // `:2,` exactly, and the last one, as io-maildir splits it: a looser split would give two names
    // differing after a colon one number.
    match name.rsplit_once(":2,") {
        Some((identity, _flags)) => identity.to_owned(),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_deliveries_a_colon_apart_are_two_messages() {
        // mbsync writes `,U=<uid>` infixes; splitting on the first colon would fold two messages
        // into one.
        let mut list = UidList::new(7);

        let first = list.uid_for("1770000000.M1,U=12:2,S");
        let second = list.uid_for("1770000000.M1,U=34:2,S");

        assert_ne!(first, second);
        assert_eq!(list.len(), 2);
    }

    const FIRST: &str = "1770000000.M1P1.host,S=42:2,S";
    const SECOND: &str = "1770000001.M2P1.host,S=91:2,";

    #[test]
    fn a_message_keeps_its_number_when_its_flags_change() {
        let mut list = UidList::new(7);
        let before = list.uid_for("1770000000.M1P1.host,S=42:2,");
        let after = list.uid_for("1770000000.M1P1.host,S=42:2,S");

        assert_eq!(before, after);
        assert_eq!(list.len(), 1, "one message, not two");
    }

    #[test]
    fn numbers_start_at_one_and_climb() {
        let mut list = UidList::new(7);
        assert_eq!(list.uid_for(FIRST), 1);
        assert_eq!(list.uid_for(SECOND), 2);
        assert_eq!(list.next(), 3, "UIDNEXT is what comes after the last");
    }

    #[test]
    fn a_deleted_message_does_not_hand_its_number_on() {
        // `UIDNEXT` never rewinds: a new message inheriting a read one's identity would arrive
        // already read.
        let mut list = UidList::new(7);
        let first = list.uid_for(FIRST);
        list.uid_for(SECOND);

        list.retain(&[SECOND.to_owned()]);
        let third = list.uid_for("1770000002.M3P1.host,S=10:2,");

        assert_ne!(third, first);
        assert_eq!(third, 3);
    }

    #[test]
    fn the_numbers_survive_being_written_and_read_back() {
        let mut list = UidList::new(1_770_000_000);
        list.uid_for(FIRST);
        list.uid_for(SECOND);

        let read = UidList::parse(&list.render()).expect("it parses");

        assert_eq!(read, list);
        assert_eq!(read.known(FIRST), Some(1));
        assert_eq!(read.next(), 3, "and the next number is not reused");
    }

    #[test]
    fn a_file_with_no_validity_line_is_not_a_uid_list() {
        assert!(UidList::parse("1 a-message\n2 another\n").is_none());
        assert!(UidList::parse("").is_none());
    }

    #[test]
    fn a_line_that_is_nonsense_costs_one_message_rather_than_the_folder() {
        let read = UidList::parse("uidvalidity 7\nnot-a-number name\n4 real-one\n\n")
            .expect("the header is there");

        assert_eq!(read.known("real-one"), Some(4));
        assert_eq!(read.len(), 1);
        assert_eq!(read.next(), 5);
    }

    #[test]
    fn an_unreadable_file_starts_again_rather_than_failing_to_open_the_folder() {
        let scratch = tempfile::tempdir().expect("a directory");
        let list = UidList::load(scratch.path(), 1_770_000_099);

        assert!(list.is_empty());
        assert_eq!(list.validity(), 1_770_000_099);
    }

    #[test]
    fn the_numbers_are_the_same_after_a_round_trip_through_a_directory() {
        let scratch = tempfile::tempdir().expect("a directory");
        let mut list = UidList::load(scratch.path(), 1_770_000_000);
        list.uid_for(FIRST);
        list.uid_for(SECOND);
        list.save(scratch.path()).expect("it writes");

        let again = UidList::load(scratch.path(), 1_770_000_999);

        assert_eq!(
            again.validity(),
            1_770_000_000,
            "the validity is not re-minted"
        );
        assert_eq!(again.known(FIRST), Some(1));
        assert_eq!(again.known(SECOND), Some(2));
    }
}
