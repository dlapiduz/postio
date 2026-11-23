//! Round trips per `Req` family, so a test can gate a budget as a count on any machine (research
//! R11).

use std::collections::BTreeMap;
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct Counts(Mutex<BTreeMap<&'static str, u64>>);

impl Counts {
    pub(crate) fn record(&self, family: &'static str) {
        *self
            .0
            .lock()
            .expect("counts are never poisoned")
            .entry(family)
            .or_default() += 1;
    }

    pub fn of(&self, family: &str) -> u64 {
        self.0
            .lock()
            .expect("counts are never poisoned")
            .get(family)
            .copied()
            .unwrap_or(0)
    }

    pub fn snapshot(&self) -> BTreeMap<&'static str, u64> {
        self.0.lock().expect("counts are never poisoned").clone()
    }
}
