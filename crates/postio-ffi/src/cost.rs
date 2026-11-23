//! The reader's cost counters for a frontend with its own web views, the same counters as
//! `postio_ui::reader::cost` (#1586).
//! Running totals, never reset: a test takes a `before` and asserts on the delta.
//! Per thread (#1390): notes and reads must come from the same thread, the main one on macOS.

use postio_ui::reader::cost;
use postio_ui::test_support;

#[uniffi::export]
pub fn note_reader_surface_created() {
    cost::note_surface_created();
}

#[uniffi::export]
pub fn note_reader_surface_released() {
    cost::note_surface_released();
}

#[uniffi::export]
pub fn note_reader_render() {
    cost::note_render();
}

#[uniffi::export]
pub fn reader_surfaces_created() -> u64 {
    test_support::surfaces_created()
}

/// Signed: a release with no matching create is a bug worth seeing, not one to floor at zero.
#[uniffi::export]
pub fn reader_surfaces_held() -> i64 {
    test_support::surfaces_held()
}

#[uniffi::export]
pub fn reader_renders_issued() -> u64 {
    test_support::renders_issued()
}
