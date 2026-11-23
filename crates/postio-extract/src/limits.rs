use std::io::{self, Read};
use std::time::Instant;

use crate::{Limit, Limits, Location, Skip, Unit};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Stop {
    Limit(Limit),
    Skip(Skip),
    Failed,
}

impl From<Limit> for Stop {
    fn from(limit: Limit) -> Self {
        Self::Limit(limit)
    }
}

pub(crate) struct Budget<'a> {
    pub(crate) limits: &'a Limits,
    pub(crate) deadline: Instant,
    text: usize,
    pub(crate) units: Vec<Unit>,
}

impl<'a> Budget<'a> {
    pub(crate) fn new(limits: &'a Limits, started: Instant) -> Self {
        Self {
            limits,
            deadline: started + limits.max_time,
            text: 0,
            units: Vec::new(),
        }
    }

    /// `Err(Time)` once the deadline has passed; walkers call it between units.
    pub(crate) fn check_time(&self) -> Result<(), Limit> {
        if Instant::now() >= self.deadline {
            Err(Limit::Time)
        } else {
            Ok(())
        }
    }

    /// Whitespace is collapsed; a whitespace-only unit is not kept but its
    /// number is still spent (page 2 stays page 2). The unit that crosses the
    /// text limit is cut at a character boundary and kept.
    pub(crate) fn push(&mut self, location: Location, raw: &str) -> Result<(), Limit> {
        self.check_time()?;
        let text = collapse(raw);
        if text.is_empty() {
            return Ok(());
        }
        if self.units.len() >= self.limits.max_units {
            return Err(Limit::Units);
        }
        let room = self.limits.max_text.saturating_sub(self.text);
        if text.len() > room {
            let cut = floor_char_boundary(&text, room);
            if cut > 0 {
                self.text += cut;
                self.units.push(Unit {
                    location,
                    text: text[..cut].to_owned(),
                });
            }
            return Err(Limit::Text);
        }
        self.text += text.len();
        self.units.push(Unit { location, text });
        Ok(())
    }
}

fn collapse(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for word in raw.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut at = at.min(text.len());
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Inflated bytes before the ratio is judged: small honest parts compress well.
const RATIO_GRACE: u64 = 1024 * 1024;

/// Stops at the entry and ratio limits whatever its header claims: the sizes
/// there are the attacker's to write.
pub(crate) struct Guarded<R> {
    inner: R,
    read: u64,
    compressed: u64,
    max_entry: u64,
    max_ratio: u64,
    deadline: Instant,
    pub(crate) tripped: Option<Limit>,
}

impl<R: Read> Guarded<R> {
    pub(crate) fn new(inner: R, compressed: u64, limits: &Limits, deadline: Instant) -> Self {
        Self {
            inner,
            read: 0,
            compressed: compressed.max(1),
            max_entry: limits.max_entry,
            max_ratio: u64::from(limits.max_ratio),
            deadline,
            tripped: None,
        }
    }

    fn trip(&mut self, limit: Limit) -> io::Error {
        self.tripped = Some(limit);
        io::Error::other("an extraction limit was reached")
    }
}

impl<R: Read> Read for Guarded<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if let Some(limit) = self.tripped {
            return Err(self.trip(limit));
        }
        if Instant::now() >= self.deadline {
            return Err(self.trip(Limit::Time));
        }
        let n = self.inner.read(buf)?;
        self.read += n as u64;
        if self.read > self.max_entry {
            return Err(self.trip(Limit::Entry));
        }
        if self.read > RATIO_GRACE && self.read / self.compressed > self.max_ratio {
            return Err(self.trip(Limit::Ratio));
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_unit_is_collapsed_and_a_blank_one_is_not_kept() {
        let limits = Limits::default();
        let mut budget = Budget::new(&limits, Instant::now());
        budget.push(Location::Line(1), "  two\n\twords ").unwrap();
        budget.push(Location::Line(2), " \n ").unwrap();
        assert_eq!(budget.units.len(), 1);
        assert_eq!(budget.units[0].text, "two words");
    }

    #[test]
    fn the_unit_that_crosses_the_text_limit_is_cut_on_a_character() {
        let limits = Limits {
            max_text: 5,
            ..Limits::default()
        };
        let mut budget = Budget::new(&limits, Instant::now());
        assert_eq!(
            budget.push(Location::Line(1), "abcdé fgh"),
            Err(Limit::Text)
        );
        assert_eq!(
            budget.units[0].text, "abcd",
            "é is two bytes and does not fit"
        );
    }

    #[test]
    fn units_past_the_limit_stop_the_walk() {
        let limits = Limits {
            max_units: 1,
            ..Limits::default()
        };
        let mut budget = Budget::new(&limits, Instant::now());
        budget.push(Location::Line(1), "one").unwrap();
        assert_eq!(budget.push(Location::Line(2), "two"), Err(Limit::Units));
    }

    #[test]
    fn a_passed_deadline_stops_the_next_unit() {
        let limits = Limits {
            max_time: Duration::ZERO,
            ..Limits::default()
        };
        let mut budget = Budget::new(&limits, Instant::now());
        assert_eq!(budget.push(Location::Line(1), "late"), Err(Limit::Time));
    }

    #[test]
    fn an_entry_that_inflates_too_well_is_stopped() {
        let limits = Limits::default();
        let zeros = std::io::repeat(0).take(4 * RATIO_GRACE);
        let mut guarded = Guarded::new(zeros, 1_000, &limits, Instant::now() + limits.max_time);
        let mut sink = Vec::new();
        assert!(guarded.read_to_end(&mut sink).is_err());
        assert_eq!(guarded.tripped, Some(Limit::Ratio));
    }

    #[test]
    fn an_entry_larger_than_the_limit_is_stopped() {
        let limits = Limits {
            max_entry: 10,
            ..Limits::default()
        };
        let mut guarded = Guarded::new(
            &[1u8; 64][..],
            64,
            &limits,
            Instant::now() + limits.max_time,
        );
        let mut sink = Vec::new();
        assert!(guarded.read_to_end(&mut sink).is_err());
        assert_eq!(guarded.tripped, Some(Limit::Entry));
    }
}
