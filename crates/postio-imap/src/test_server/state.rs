//! What the server knows: mailboxes, messages, and the sequence numbers that
//! make them addressable.
//!
//! One `ServerState` is shared by every connection behind a mutex, because
//! that is what a mail server is: two clients selecting the same mailbox see
//! each other's changes. Nothing here awaits, so the lock is only ever held
//! for the length of a command.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use postio_model::{Flag, FlagSet, ModSeq, UidValidity};

use super::{Fault, Quirk};

/// A message as the server holds it.
#[derive(Clone, Debug)]
pub(super) struct Message {
    pub(super) uid: u32,
    pub(super) raw: Vec<u8>,
    pub(super) flags: FlagSet,
    pub(super) internal_date: DateTime<Utc>,
    pub(super) mod_seq: u64,
}

/// A mailbox and its UID space.
#[derive(Clone, Debug)]
pub(super) struct Mailbox {
    pub(super) path: String,
    pub(super) delimiter: char,
    pub(super) attributes: Vec<String>,
    pub(super) subscribed: bool,
    pub(super) uid_validity: u32,
    pub(super) uid_next: u32,
    pub(super) highest_mod_seq: u64,
    pub(super) messages: Vec<Message>,
    /// What was removed, and at which modification sequence, so that a
    /// `SELECT (QRESYNC …)` can report `VANISHED (EARLIER)` for a client that
    /// was away when it happened.
    pub(super) vanished: Vec<(u32, u64)>,
}

impl Mailbox {
    /// Advances the modification sequence and returns the new value.
    ///
    /// RFC 7162 §3.1.2.1: every change gets a value strictly greater than the
    /// mailbox's previous HIGHESTMODSEQ, so `CHANGEDSINCE` — which is
    /// strictly greater-than — can see it.
    pub(super) fn bump(&mut self) -> u64 {
        self.highest_mod_seq += 1;
        self.highest_mod_seq
    }

    /// Adds a message arriving now, stamping it with a fresh sequence.
    pub(super) fn push(&mut self, raw: Vec<u8>, flags: FlagSet, at: DateTime<Utc>) -> u32 {
        let uid = self.uid_next;
        self.uid_next += 1;
        let mod_seq = self.bump();
        self.messages.push(Message {
            uid,
            raw,
            flags,
            internal_date: at,
            mod_seq,
        });
        uid
    }

    /// Removes a message, remembering it for QRESYNC.
    pub(super) fn remove(&mut self, uid: u32) -> bool {
        let Some(index) = self.messages.iter().position(|message| message.uid == uid) else {
            return false;
        };
        self.messages.remove(index);
        let mod_seq = self.bump();
        self.vanished.push((uid, mod_seq));
        true
    }

    /// Renumbers the UID space, as a server restored from backup does.
    ///
    /// Everything a client believed about this mailbox is now wrong, which is
    /// the whole point of the generation counter: UIDs restart at 1 and the
    /// vanished list is meaningless across the boundary.
    pub(super) fn renumber(&mut self, uid_validity: u32) {
        self.uid_validity = uid_validity;
        self.uid_next = 1;
        self.vanished.clear();
        for index in 0..self.messages.len() {
            self.messages[index].uid = self.uid_next;
            self.uid_next += 1;
        }
        self.bump();
    }

    pub(super) fn find(&self, uid: u32) -> Option<&Message> {
        self.messages.iter().find(|message| message.uid == uid)
    }

    pub(super) fn find_mut(&mut self, uid: u32) -> Option<&mut Message> {
        self.messages.iter_mut().find(|message| message.uid == uid)
    }

    pub(super) fn uids(&self) -> Vec<u32> {
        self.messages.iter().map(|message| message.uid).collect()
    }

    pub(super) fn highest_uid(&self) -> u32 {
        self.messages
            .last()
            .map(|message| message.uid)
            .unwrap_or(self.uid_next.saturating_sub(1))
    }

    pub(super) fn unseen(&self) -> u32 {
        self.messages
            .iter()
            .filter(|message| message.flags.is_unread())
            .count() as u32
    }
}

/// Everything the server is, shared by every connection.
#[derive(Debug)]
pub(super) struct ServerState {
    pub(super) account: String,
    pub(super) password: String,
    /// What the greeting advertises. Short by default: the provider Postio
    /// targets hides every extension until you have logged in.
    pub(super) banner: Vec<String>,
    /// What `CAPABILITY` reports once authenticated.
    pub(super) capabilities: Vec<String>,
    pub(super) mailboxes: Vec<Mailbox>,
    pub(super) quirks: BTreeSet<Quirk>,
    /// Armed faults, each fired once by the first command that matches it.
    pub(super) faults: Vec<Fault>,
    /// Every command line the server has been sent, tags included.
    pub(super) log: Vec<String>,
}

impl ServerState {
    pub(super) fn index_of(&self, path: &str) -> Option<usize> {
        self.mailboxes
            .iter()
            .position(|mailbox| mailbox.path.eq_ignore_ascii_case(path))
    }

    pub(super) fn mailbox(&self, path: &str) -> Option<&Mailbox> {
        self.index_of(path).map(|index| &self.mailboxes[index])
    }

    pub(super) fn mailbox_mut(&mut self, path: &str) -> Option<&mut Mailbox> {
        self.index_of(path)
            .map(move |index| &mut self.mailboxes[index])
    }

    pub(super) fn has(&self, quirk: Quirk) -> bool {
        self.quirks.contains(&quirk)
    }

    pub(super) fn supports(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|name| name.eq_ignore_ascii_case(capability))
    }

    /// Takes the first armed fault that matches `command`, if any.
    pub(super) fn take_fault(&mut self, command: &str) -> Option<Fault> {
        let index = self
            .faults
            .iter()
            .position(|fault| fault.matches(command))?;
        Some(self.faults.remove(index))
    }
}

/// Parses one wire flag, e.g. `\Seen` or `$label`.
pub(super) fn flag(raw: &str) -> Flag {
    Flag::parse(raw)
}

/// Renders a flag set the way a server writes one.
pub(super) fn flag_list(flags: &FlagSet) -> String {
    flags.iter().map(Flag::as_str).collect::<Vec<_>>().join(" ")
}

/// The `INTERNALDATE` spelling: `"07-Feb-1994 21:52:25 +0000"`.
pub(super) fn internal_date(at: DateTime<Utc>) -> String {
    at.format("%d-%b-%Y %H:%M:%S %z").to_string()
}

/// Seeds a mailbox's starting state.
pub(super) fn seed(
    path: String,
    delimiter: char,
    attributes: Vec<String>,
    subscribed: bool,
    uid_validity: UidValidity,
    highest_mod_seq: ModSeq,
    messages: Vec<(Vec<u8>, FlagSet, Option<DateTime<Utc>>)>,
) -> Mailbox {
    let mut mailbox = Mailbox {
        path,
        delimiter,
        attributes,
        subscribed,
        uid_validity: uid_validity.get(),
        uid_next: 1,
        highest_mod_seq: highest_mod_seq.get(),
        messages: Vec::new(),
        vanished: Vec::new(),
    };

    // A mailbox at rest: every message already carries the mailbox's starting
    // modification sequence, and nothing has happened since.
    for (raw, flags, at) in messages {
        let uid = mailbox.uid_next;
        mailbox.uid_next += 1;
        mailbox.messages.push(Message {
            uid,
            internal_date: at.unwrap_or_else(|| date_header(&raw).unwrap_or_else(Utc::now)),
            raw,
            flags,
            mod_seq: mailbox.highest_mod_seq,
        });
    }

    mailbox
}

/// The `Date` header, for an `INTERNALDATE` nobody set explicitly.
fn date_header(raw: &[u8]) -> Option<DateTime<Utc>> {
    let text = String::from_utf8_lossy(&raw[..raw.len().min(8 * 1024)]);
    for line in text.split("\r\n") {
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Date:") {
            return DateTime::parse_from_rfc2822(value.trim())
                .ok()
                .map(|at| at.with_timezone(&Utc));
        }
    }
    None
}

/// The UID a sequence-set token names, resolving `*` against `highest`.
pub(super) fn resolve(token: &str, highest: u32) -> Option<u32> {
    if token == "*" {
        return Some(highest);
    }
    token.parse().ok()
}

/// Whether `value` falls inside an RFC 3501 sequence set such as `1:3,7,9:*`.
pub(super) fn in_sequence_set(set: &str, value: u32, highest: u32) -> bool {
    set.split(',').any(|range| match range.split_once(':') {
        None => resolve(range, highest) == Some(value),
        Some((first, last)) => {
            let Some(first) = resolve(first, highest) else {
                return false;
            };
            let Some(last) = resolve(last, highest) else {
                return false;
            };
            let (low, high) = if first <= last {
                (first, last)
            } else {
                (last, first)
            };
            (low..=high).contains(&value)
        }
    })
}

/// Coalesces UIDs into the `1:3,7` spelling a `VANISHED` response uses.
pub(super) fn sequence_set_of(uids: &BTreeSet<u32>) -> String {
    let mut ranges: Vec<String> = Vec::new();
    let mut iter = uids.iter().copied();
    let Some(mut start) = iter.next() else {
        return String::new();
    };
    let mut end = start;

    for uid in iter {
        if uid == end + 1 {
            end = uid;
            continue;
        }
        ranges.push(render_range(start, end));
        start = uid;
        end = uid;
    }
    ranges.push(render_range(start, end));
    ranges.join(",")
}

fn render_range(start: u32, end: u32) -> String {
    if start == end {
        format!("{start}")
    } else {
        format!("{start}:{end}")
    }
}
