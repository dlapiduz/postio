//! When the cursor has rested on a message long enough for it to count as
//! read (#71).
//!
//! One number and one rule, shared, because both frontends need them and
//! neither should choose for itself. The rule protects a signal that cannot
//! be recovered once it is gone: unread state is only useful while it means
//! "you have not looked at this", and a client that marks on arrival destroys
//! it the first time somebody scrolls a mailbox end to end.
//!
//! It lived in `postio-gtk::list_view` until #1159, which is where the macOS
//! frontend could not reach it — so that one marked nothing at all, ever, and
//! a Mac's inbox count never moved. A second implementation would have been a
//! second answer to "how long is long enough", on the one rule where being
//! wrong deletes something.
//!
//! The *timer* stays with each toolkit: `glib::timeout_add_local_once` on one
//! side and a `DispatchWorkItem` on the other. What is shared is the delay
//! and the arming rule, which is all that has a decision in it.

use std::time::Duration;

/// How long the cursor rests on a message before it counts as read (#71).
///
/// Marking on arrival is what this number exists to avoid: scrolling from one
/// end of a mailbox to the other passes over every message in between, and
/// marking all of them destroys the unread state as a signal — the one thing
/// it is for. A dwell means "the cursor stayed here long enough that a person
/// could have read it".
///
/// A second is about the shortest value that cleanly separates the two
/// gestures. A held `j` repeats roughly every 30ms, so a sweep of fifty
/// messages rests nowhere and marks nothing; reading deliberately, even
/// quickly, leaves the cursor still for longer than this on anything worth
/// looking at. Much shorter starts catching the sweep, and much longer leaves
/// mail you plainly read still bold, which reads as the app not keeping up.
pub const DWELL_TO_READ: Duration = Duration::from_millis(1_000);

/// What a frontend should do about a cursor that has just moved.
///
/// The arming rule *and* the delay, together, deliberately. Handing them over
/// separately is what made the first version of this dead code: a frontend
/// given only the number wrote the rule again beside it, and a rule with no
/// caller cannot keep anything in step. [`Start`](Arm::Start) carries how long
/// to wait, so a frontend cannot arm a clock without having asked.
///
/// Everything hard about a dwell is still the *timer*, and a timer belongs to
/// whichever toolkit owns the run loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arm {
    /// Cancel whatever is running and start a clock on this message.
    Start {
        /// The message the cursor landed on.
        message: i64,
        /// How long to wait before it counts as read. [`DWELL_TO_READ`],
        /// carried rather than looked up so the two arrive together.
        after: Duration,
    },
    /// Cancel whatever is running and start nothing.
    ///
    /// The cursor is on no message, or on a row whose page has not arrived —
    /// there is nothing to mark, and an armed clock would fire at whatever
    /// happened to be there when it did.
    Cancel,
}

/// What to do when the cursor lands on `message`.
///
/// `None` means the cursor is nowhere, or on a row still being read. Both
/// cancel: a clock armed against an unknown message would mark whichever one
/// arrived first, which is the sweep problem wearing a different hat.
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
        // Not "arm and hope": a clock with no message would mark whatever
        // the cursor happened to be on when it fired, which is exactly the
        // sweep this rule exists to prevent.
        assert_eq!(on_cursor(None), Arm::Cancel);
    }

    #[test]
    fn the_delay_separates_a_sweep_from_a_read() {
        // A held `j` repeats about every 30ms. The delay has to be far enough
        // above that for a sweep to rest nowhere, and low enough that mail
        // somebody plainly read does not stay bold.
        assert!(DWELL_TO_READ > Duration::from_millis(300));
        assert!(DWELL_TO_READ <= Duration::from_millis(2_000));
    }
}
