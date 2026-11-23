//! The dwell rule as one value, from `postio_ui::dwell`: resting marks, sweeping does not (#71).

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DwellArmFfi {
    Start {
        message: i64,
        /// Milliseconds, because uniffi has no `Duration`.
        milliseconds: u64,
    },
    Cancel,
}

/// `None` means the cursor is nowhere or on a row whose page has not arrived; both cancel.
/// A free function: it reads no session state.
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
