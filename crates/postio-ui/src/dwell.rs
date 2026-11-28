//! The dwell delay and arming rule for marking a message read, shared by both frontends (#71).
//! The timer stays with each toolkit.

use std::time::Duration;

/// Long enough that a held `j` sweep (about 30 ms a repeat) rests nowhere and marks nothing,
/// short enough that deliberately read mail does not stay bold.
pub const DWELL_TO_READ: Duration = Duration::from_millis(1_000);

/// The arming rule and the delay together, so a frontend cannot arm a clock without asking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    Start {
        message: i64,
        after: Duration,
    },
    /// The cursor is on no message or on a row not yet loaded; an armed clock would fire at
    /// whatever is there then.
    Cancel,
}

pub fn on_cursor(message: Option<i64>) -> Arm {
    match message {
        Some(message) => Arm::Start {
            message,
            after: DWELL_TO_READ,
        },
        None => Arm::Cancel,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landing_on_a_message_starts_a_clock_on_that_message() {
        assert_eq!(
            on_cursor(Some(7)),
            Arm::Start {
                message: 7,
                after: DWELL_TO_READ
            }
        );
    }

    #[test]
    fn a_cursor_on_nothing_arms_nothing() {
        assert_eq!(on_cursor(None), Arm::Cancel);
    }

    #[test]
    fn the_delay_separates_a_sweep_from_a_read() {
        assert!(DWELL_TO_READ > Duration::from_millis(300));
        assert!(DWELL_TO_READ <= Duration::from_millis(2_000));
    }
}
