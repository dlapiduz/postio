//! A set of message UIDs, in the compact form a server wants to be told.
//!
//! IMAP addresses messages with sequence sets — `1:3,7,10:*` — and a client
//! that sends ten thousand comma-separated numbers instead of a range is both
//! slower and, on some servers, over the command-length limit. This type keeps
//! UIDs coalesced from the moment they are collected, so no caller has to
//! remember to.
//!
//! It is also where batching lives: [`UidSet::chunks`] splits a set into
//! fetch-sized pieces without losing or reordering anything, which is what
//! keeps a ten-thousand-message initial sync off the heap.

use std::fmt;

use postio_model::Uid;

/// The `*` in a sequence set: "the highest UID in the mailbox, whatever it is".
///
/// Representing it as the largest possible UID rather than a separate variant
/// keeps the range arithmetic in one shape. No real message can have this UID —
/// a server that assigned it would have nowhere to go next.
const STAR: u32 = u32::MAX;

/// A set of UIDs, kept sorted, coalesced and free of zeroes.
///
/// # Invariants
///
/// * Ranges are ascending, non-overlapping and non-adjacent — `1,2,3` is
///   stored, and rendered, as `1:3`.
/// * UID `0` is never a member. IMAP UIDs start at 1, so a zero arriving here
///   is a bug upstream and is dropped rather than propagated into a command.
/// * A set is *open-ended* when it reaches [`STAR`]; it then has no known size.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UidSet {
    ranges: Vec<(u32, u32)>,
}

impl UidSet {
    /// The empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Every message in the mailbox: `1:*`.
    pub fn all() -> Self {
        Self::from_uid_onwards(Uid::new(1))
    }

    /// A single UID.
    pub fn single(uid: Uid) -> Self {
        let mut set = Self::new();
        set.insert(uid);
        set
    }

    /// An inclusive range. Reversed bounds are accepted and normalized.
    pub fn range(first: Uid, last: Uid) -> Self {
        let mut set = Self::new();
        set.insert_range(first, last);
        set
    }

    /// `uid:*` — from `uid` to whatever the highest UID turns out to be.
    pub fn from_uid_onwards(uid: Uid) -> Self {
        Self::range(uid, Uid::new(STAR))
    }

    /// Adds one UID. A zero is ignored.
    pub fn insert(&mut self, uid: Uid) {
        self.insert_range(uid, uid);
    }

    /// Adds an inclusive range. Reversed bounds are accepted and normalized.
    pub fn insert_range(&mut self, first: Uid, last: Uid) {
        let (low, high) = if first.get() <= last.get() {
            (first.get(), last.get())
        } else {
            (last.get(), first.get())
        };
        // UIDs start at 1; clamp rather than reject so a `0:5` from a sloppy
        // caller still means "the first five messages".
        let low = low.max(1);
        if high < 1 {
            return;
        }
        self.ranges.push((low, high));
        self.normalize();
    }

    /// Whether the set holds no UIDs at all.
    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    /// Whether the set ends in `*` and therefore has no known size.
    pub fn is_open_ended(&self) -> bool {
        self.ranges.last().is_some_and(|(_, high)| *high == STAR)
    }

    /// Whether `uid` is a member.
    pub fn contains(&self, uid: Uid) -> bool {
        let uid = uid.get();
        uid >= 1
            && self
                .ranges
                .iter()
                .any(|(low, high)| *low <= uid && uid <= *high)
    }

    /// How many UIDs the set holds, or `None` when it is open-ended.
    pub fn len(&self) -> Option<u64> {
        if self.is_open_ended() {
            return None;
        }
        Some(
            self.ranges
                .iter()
                .map(|(low, high)| u64::from(*high) - u64::from(*low) + 1)
                .sum(),
        )
    }

    /// The ranges, as inclusive `(first, last)` pairs.
    pub fn ranges(&self) -> impl Iterator<Item = (Uid, Uid)> + '_ {
        self.ranges
            .iter()
            .map(|(low, high)| (Uid::new(*low), Uid::new(*high)))
    }

    /// Every UID in the set, ascending.
    ///
    /// An open-ended set has no last element, so iteration stops where the `*`
    /// begins. Call [`is_open_ended`](Self::is_open_ended) first if that
    /// distinction matters to the caller.
    pub fn uids(&self) -> impl Iterator<Item = Uid> + '_ {
        self.ranges
            .iter()
            .filter(|(_, high)| *high != STAR)
            .flat_map(|(low, high)| (*low..=*high).map(Uid::new))
    }

    /// Splits the set into pieces of at most `max` UIDs each, in order.
    ///
    /// This is how a large fetch is kept off the heap: the caller fetches one
    /// chunk, writes it, reports progress, and only then asks for the next.
    /// Ranges are split rather than kept whole, so `1:10` chunked by four is
    /// `1:4`, `5:8`, `9:10` and not one oversized piece.
    ///
    /// An open-ended set cannot be split — nobody knows how many UIDs `10:*`
    /// covers — so it comes back as a single chunk, unchanged.
    pub fn chunks(&self, max: usize) -> Vec<UidSet> {
        if self.is_empty() {
            return Vec::new();
        }
        if self.is_open_ended() {
            return vec![self.clone()];
        }

        let max = u64::from(u32::try_from(max.max(1)).unwrap_or(u32::MAX));
        let mut chunks = Vec::new();
        let mut current = UidSet::new();
        let mut room = max;

        for (low, high) in &self.ranges {
            let mut cursor = u64::from(*low);
            let end = u64::from(*high);
            while cursor <= end {
                let take = room.min(end - cursor + 1);
                let last = cursor + take - 1;
                current
                    .ranges
                    .push((cursor as u32, u32::try_from(last).unwrap_or(STAR)));
                cursor = last + 1;
                room -= take;
                if room == 0 {
                    chunks.push(std::mem::take(&mut current));
                    room = max;
                }
            }
        }

        if !current.is_empty() {
            chunks.push(current);
        }
        chunks
    }

    /// The set as an IMAP sequence set: `1:3,7,10:*`.
    ///
    /// The empty set renders as the empty string; it is the caller's job not
    /// to send a command with no messages in it.
    pub fn to_sequence_set(&self) -> String {
        let mut out = String::new();
        for (index, (low, high)) in self.ranges.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            match (low, high) {
                (low, high) if low == high => out.push_str(&low.to_string()),
                (low, &STAR) => {
                    out.push_str(&low.to_string());
                    out.push_str(":*");
                }
                (low, high) => {
                    out.push_str(&low.to_string());
                    out.push(':');
                    out.push_str(&high.to_string());
                }
            }
        }
        out
    }

    /// Sorts, merges overlaps, and joins ranges that touch.
    fn normalize(&mut self) {
        self.ranges.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::with_capacity(self.ranges.len());
        for (low, high) in self.ranges.drain(..) {
            match merged.last_mut() {
                // `saturating_add` so a range ending at u32::MAX does not wrap
                // while we test for adjacency.
                Some(last) if low <= last.1.saturating_add(1) => last.1 = last.1.max(high),
                _ => merged.push((low, high)),
            }
        }
        self.ranges = merged;
    }
}

impl fmt::Display for UidSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_sequence_set())
    }
}

impl FromIterator<Uid> for UidSet {
    fn from_iter<I: IntoIterator<Item = Uid>>(iter: I) -> Self {
        let mut set = Self::new();
        set.ranges = iter
            .into_iter()
            .map(Uid::get)
            .filter(|uid| *uid >= 1)
            .map(|uid| (uid, uid))
            .collect();
        set.normalize();
        set
    }
}

impl Extend<Uid> for UidSet {
    fn extend<I: IntoIterator<Item = Uid>>(&mut self, iter: I) {
        for uid in iter {
            self.insert(uid);
        }
    }
}
