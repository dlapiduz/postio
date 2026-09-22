//! The reader's cost counters, for a frontend that owns its own web views.
//!
//! `postio-gtk` notes a surface's creation and release, and each document it
//! hands to one, by calling [`postio_ui::reader::cost`] directly. The macOS
//! reader is a `WKWebView` this crate never sees, so those notes have to cross
//! (#1586) — and they cross into **the same counters**, not a second set: the
//! point of counting is one definition of what moving between messages costs,
//! measured on both platforms.
//!
//! The readers are here for the Swift tests, which cannot link
//! `postio_ui::test_support` any other way. They are running totals, never
//! reset: a test takes a `before`, acts, and asserts on the delta.
//!
//! **Per thread** (#1390). A frontend notes from the thread that owns its
//! interface, which on macOS is the main thread; a test that reads from any
//! other thread reads that thread's zeros. `@MainActor` on the test is what
//! makes the read and the notes the same counters.

use postio_ui::reader::cost;
use postio_ui::test_support;

/// A reader surface — one `WKWebView` — was created.
#[uniffi::export]
pub fn note_reader_surface_created() {
    cost::note_surface_created();
}

/// A reader surface was released: the web view, and the content process
/// behind it, can go.
#[uniffi::export]
pub fn note_reader_surface_released() {
    cost::note_surface_released();
}

/// A document was handed to a reader surface, at the frontend's one load
/// choke point.
#[uniffi::export]
pub fn note_reader_render() {
    cost::note_render();
}

/// How many reader surfaces this thread has noted creating.
#[uniffi::export]
pub fn reader_surfaces_created() -> u64 {
    test_support::surfaces_created()
}

/// Reader surfaces noted as created and not yet released.
///
/// Signed for the reason the shared reader gives: a frontend that reports a
/// release it never created is a bug worth seeing, not one to floor at zero.
#[uniffi::export]
pub fn reader_surfaces_held() -> i64 {
    test_support::surfaces_held()
}

/// How many documents this thread has noted handing to a reader surface.
#[uniffi::export]
pub fn reader_renders_issued() -> u64 {
    test_support::renders_issued()
}
