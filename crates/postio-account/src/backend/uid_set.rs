//! Coalesced from collection (`1:3,7,10:*`), since ten thousand comma-separated UIDs can overrun a
//! server's command-length limit; `chunks` batches fetches.

use std::fmt;

use postio_model::Uid;

/// The largest UID stands for `*`, keeping range arithmetic in one shape; no real message has it.
const STAR: u32 = u32::MAX;

/// Invariants: ranges ascending, non-overlapping and non-adjacent; UID 0 is dropped as an upstream
/// bug; reaching `STAR` makes the set open-ended with no known size.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct UidSet {
    ranges: Vec<(u32, u32)>,
}

impl UidSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn all() -> Self {
        Self::from_uid_onwards(Uid::new(1))
    }

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

    pub fn from_uid_onwards(uid: Uid) -> Self {
        Self::range(uid, Uid::new(STAR))
    }

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

    pub fn is_empty(&self) -> bool {
        self.ranges.is_empty()
    }

    pub fn is_open_ended(&self) -> bool {
        self.ranges.last().is_some_and(|(_, high)| *high == STAR)
    }

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

    pub fn ranges(&self) -> impl Iterator<Item = (Uid, Uid)> + '_ {
        self.ranges
            .iter()
            .map(|(low, high)| (Uid::new(*low), Uid::new(*high)))
    }

    /// An open-ended set stops iterating where the `*` begins.
    pub fn uids(&self) -> impl Iterator<Item = Uid> + '_ {
        self.ranges
            .iter()
            .filter(|(_, high)| *high != STAR)
            .flat_map(|(low, high)| (*low..=*high).map(Uid::new))
    }

    /// Ranges are split, so `1:10` by four is `1:4`, `5:8`, `9:10`. An open-ended set comes back as
    /// one chunk, unchanged.
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

    /// The empty set renders as the empty string; callers must not send a command with no messages.
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
