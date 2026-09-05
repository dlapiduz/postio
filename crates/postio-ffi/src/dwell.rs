//! The dwell rule, as a frontend asks it.
//!
//! `postio_ui::dwell` owns "resting marks, sweeping does not" (#71) and how
//! long resting is. This carries the answer across, as one value rather than
//! as a number the caller then writes a rule around — which is what the first
//! version of #1159 did, leaving the rule with three copies and no caller.

/// What to do about a cursor that has just moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DwellArmFfi {
    /// Cancel whatever is running and start a clock on this message.
    Start {
        /// The message the cursor landed on.
        message: i64,
        /// How long to wait before it counts as read. Milliseconds, because
        /// uniffi has no `Duration`; the frontend turns it into whatever its
        /// own timer takes.
        milliseconds: u64,
    },
    /// Cancel whatever is running and start nothing.
    Cancel,
}

/// What a frontend should do now that the cursor is on `message`.
///
/// `None` means the cursor is nowhere, or on a row whose page has not
/// arrived. Both cancel: a clock armed against an unknown message would mark
/// whichever one turned up, which is the sweep problem wearing a different
/// hat.
///
/// A free function, not a `Session` method: it reads no session state, and a
/// frontend should be able to ask it before a store is even open.
#[uniffi::export]
pub fn dwell_on_cursor(message: Option<i64>) -> DwellArmFfi {
    match postio_ui::dwell::on_cursor(message) {
        postio_ui::dwell::Arm::Start { message, after } => DwellArmFfi::Start {
            message,
            milliseconds: after.as_millis() as u64,
        },
        postio_ui::dwell::Arm::Cancel => DwellArmFfi::Cancel,
    }
}
