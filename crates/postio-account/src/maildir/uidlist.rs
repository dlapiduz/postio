//! Stable UIDs for a maildir, kept beside it (#1278).
//!
//! Postio's whole sync machinery is written against a server that hands out
//! `UIDVALIDITY` and monotonic `UID`s: the list keys on them, a resync asks
//! "what is new since UID n", and a second pass over the same mail is cheap
//! precisely because the identity of a message is a number that does not
//! move. A maildir has none of that. It has filenames.
//!
//! So this invents the numbers and **writes them down**, which is what every
//! tool that has solved this before does (mbsync's `.uidvalidity`,
//! offlineimap's uid map). Deriving them instead — sorting filenames and
//! counting — would renumber the whole mailbox the first time a message was
//! delivered or deleted, and every message in Postio's store would then point
//! at the wrong file.
//!
//! # What is written
//!
//! One file per maildir folder, `.postio-uids`, one entry per line:
//!
//! ```text
//! uidvalidity 1770000000
//! 1 1770000000.M1P1.host,S=42:2,S
//! 2 1770000001.M2P1.host,S=91:2,
//! ```
//!
//! The **name without its flags**, because maildir flags live in the
//! filename: marking a message read renames the file, and a map keyed on the
//! whole name would lose the message and invent a new one for it. That is
//! the dedupe key the composer's own history warned about, in the one place
//! it actually bites.
//!
//! Numbers are never reused. A deleted message's UID stays spent, which is
//! what `UIDNEXT` means and what stops a new message inheriting the identity
//! of one somebody has already read.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The file a folder's UIDs are kept in.
const FILE: &str = ".postio-uids";

/// A maildir folder's UIDs, as read from disk and as they will be written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UidList {
    /// What Postio calls this folder's `UIDVALIDITY`.
    ///
    /// Minted once, from the clock, and never changed while the file
    /// survives. If it is ever lost the new value tells the store to treat
    /// every message as new — which is the honest answer, because the
    /// mapping that said otherwise is gone.
    validity: u32,
    /// Name-without-flags to UID.
    uids: BTreeMap<String, u32>,
    /// The next UID to hand out. Monotonic, never rewound.
    next: u32,
}

impl UidList {
    /// An empty list that will start numbering at one.
    pub fn new(validity: u32) -> Self {
        Self {
            validity,
            uids: BTreeMap::new(),
            next: 1,
        }
    }

    /// Read the list beside `folder`, or start one.
    ///
    /// **Unreadable is empty, not an error.** A truncated or hand-edited file
    /// means the mapping is gone, and the recovery — a fresh validity, so the
    /// store re-reads everything — is the same one a server forces when its
    /// own `UIDVALIDITY` changes. Failing to open the folder instead would
    /// make a corrupt sidecar into unreachable mail.
    pub fn load(folder: &Path, now: u32) -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path(folder)) else {
            return Self::new(now);
        };
        Self::parse(&text).unwrap_or_else(|| Self::new(now))
    }

    /// Parse the file's text; `None` if it does not begin with a validity,
    /// which is the one line that cannot be guessed at.
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
            // A line naming nothing is skipped rather than fatal: what it
            // would cost is one message renumbered, against a whole folder
            // renumbered for refusing the file.
            if name.is_empty() {
                continue;
            }
            list.uids.insert(name.to_owned(), uid);
            list.next = list.next.max(uid + 1);
        }
        Some(list)
    }

    /// The file's text, ready to write.
    pub fn render(&self) -> String {
        let mut out = format!("uidvalidity {}\n", self.validity);
        // Sorted by UID rather than by name, so the file reads as the history
        // it is and a diff between two runs is the messages that arrived.
        let mut entries: Vec<(&String, &u32)> = self.uids.iter().collect();
        entries.sort_by_key(|(_, uid)| **uid);
        for (name, uid) in entries {
            out.push_str(&format!("{uid} {name}\n"));
        }
        out
    }

    /// Write it beside `folder`.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        std::fs::write(Self::path(folder), self.render())
    }

    /// This folder's `UIDVALIDITY`.
    pub fn validity(&self) -> u32 {
        self.validity
    }

    /// The next UID that will be handed out — IMAP's `UIDNEXT`.
    pub fn next(&self) -> u32 {
        self.next
    }

    /// The UID for `name`, assigning one if this is the first sight of it.
    ///
    /// `name` is the filename with its `:2,flags` suffix removed: marking a
    /// message read renames the file, and keying on the whole name would
    /// lose the message and mint a second UID for it.
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

    /// The UID `name` already has, if it has one.
    pub fn known(&self, name: &str) -> Option<u32> {
        self.uids.get(&strip_flags(name)).copied()
    }

    /// Forget the names that are no longer in the folder.
    ///
    /// Their numbers stay spent: `next` is never rewound, so a message
    /// delivered later cannot inherit the identity of one somebody has
    /// already read and archived.
    pub fn retain(&mut self, present: &[String]) {
        let keys: std::collections::BTreeSet<String> =
            present.iter().map(|name| strip_flags(name)).collect();
        self.uids.retain(|name, _| keys.contains(name));
    }

    /// How many messages this folder is holding numbers for.
    pub fn len(&self) -> usize {
        self.uids.len()
    }

    /// Whether it holds none.
    pub fn is_empty(&self) -> bool {
        self.uids.is_empty()
    }

    fn path(folder: &Path) -> PathBuf {
        folder.join(FILE)
    }
}

/// A maildir filename without its `:2,<flags>` suffix.
///
/// The suffix is the flags, and the flags change: `S` is added when a message
/// is read, `F` when it is flagged. Everything before the colon is the
/// delivery identity and does not move.
fn strip_flags(name: &str) -> String {
    match name.split_once(':') {
        Some((identity, _)) => identity.to_owned(),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two deliveries, as a maildir names them: an identity, then flags.
    const FIRST: &str = "1770000000.M1P1.host,S=42:2,S";
    const SECOND: &str = "1770000001.M2P1.host,S=91:2,";

    #[test]
    fn a_message_keeps_its_number_when_its_flags_change() {
        // Marking a message read *renames the file*. A map keyed on the whole
        // name would lose it and mint a second UID — the same message twice
        // in Postio's store, one of them unreachable.
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
        // What `UIDNEXT` means, and the reason it must never rewind: a new
        // message inheriting a read message's identity is a message that
        // arrives already read, in a list that will not show it as new.
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
        // The one line that cannot be guessed at: without it there is no way
        // to say whether these numbers describe this folder.
        assert!(UidList::parse("1 a-message\n2 another\n").is_none());
        assert!(UidList::parse("").is_none());
    }

    #[test]
    fn a_line_that_is_nonsense_costs_one_message_rather_than_the_folder() {
        // Renumbering a whole folder because one line was truncated would
        // re-download everything; skipping the line renumbers one message.
        let read = UidList::parse("uidvalidity 7\nnot-a-number name\n4 real-one\n\n")
            .expect("the header is there");

        assert_eq!(read.known("real-one"), Some(4));
        assert_eq!(read.len(), 1);
        assert_eq!(read.next(), 5);
    }

    #[test]
    fn an_unreadable_file_starts_again_rather_than_failing_to_open_the_folder() {
        // A corrupt sidecar must not become unreachable mail. The recovery is
        // the one a server forces when its own UIDVALIDITY changes.
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
