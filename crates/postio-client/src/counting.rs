//! How many round trips a frontend has made, by kind.
//!
//! Principle V gates budgets as counts, and a process boundary adds a cost
//! the storage seam's statement counter cannot see: the round trip itself.
//! This is that count, per [`Req`](crate::protocol::Req) family, so a test
//! can say "a list keystroke costs at most one `Page` and no `Body`" and mean
//! it on any machine (research R11).

use std::collections::BTreeMap;
use std::sync::Mutex;

/// Round trips made through one [`Client`](crate::Client), by family.
#[derive(Debug, Default)]
pub struct Counts(Mutex<BTreeMap<&'static str, u64>>);

impl Counts {
    /// Record one round trip of `family`.
    pub(crate) fn record(&self, family: &'static str) {
        *self
            .0
            .lock()
            .expect("counts are never poisoned")
            .entry(family)
            .or_default() += 1;
    }

    /// How many round trips of `family` have been made.
    pub fn of(&self, family: &str) -> u64 {
        self.0
            .lock()
            .expect("counts are never poisoned")
            .get(family)
            .copied()
            .unwrap_or(0)
    }

    /// Every family seen so far and its count, for a failure message.
    pub fn snapshot(&self) -> BTreeMap<&'static str, u64> {
        self.0.lock().expect("counts are never poisoned").clone()
    }
}
