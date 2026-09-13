//! Answering a synchronous callback that has to read the store, now.
//!
//! # Why this exists
//!
//! Some callbacks cannot await. A GTK widget asks "what is this recipient's
//! name", "which mailbox is this row in", "what is the default signature" —
//! and needs the answer before it can lay out. WebKit asks a `cid:` URI to
//! resolve to bytes in the middle of laying out a document. There is nothing
//! to hand a future to.
//!
//! It lives here rather than in `postio-app` because `postio-session` has the
//! same problem in `reading::cid_source`, and two copies of the
//! `Handle::try_current` dance is two chances to write only the second half of
//! it — which is the bug this module is mostly about.
//!
//! # Why it is not a regression
//!
//! These reads blocked before as well: the storage layer was synchronous and
//! these callbacks called it directly on the GTK thread. What changed is the
//! spelling, not the cost — the same indexed read, through an async API, with
//! a runtime to turn the future back into a value.
//!
//! # What must not come through here
//!
//! Anything that could take real time. The budget is unchanged: interaction
//! under 16 ms, and a callback that blocks the main loop for longer than that
//! is a dropped frame whatever engine is underneath. Work that is not a
//! bounded indexed read belongs on a spawned task reporting through an event,
//! which is what every other path in this crate does.

use std::cell::OnceCell;
use std::future::Future;

thread_local! {
    /// One runtime per thread, built on first use.
    ///
    /// `current_thread`: there is one caller, it is already on the thread it
    /// wants to be on, and a worker pool would be a pool for nothing.
    static BRIDGE: OnceCell<tokio::runtime::Runtime> = const { OnceCell::new() };
}

/// Run `future` to completion on this thread, blocking until it answers.
///
/// # Panics
///
/// If a runtime cannot be built at all, which means the process has run out
/// of the file descriptors a reactor needs. Nothing this callback could
/// return would be true in that case.
pub fn now<T>(future: impl Future<Output = T>) -> T {
    // Already on a runtime thread -- which the application never is, because
    // GTK owns this thread, but the tests are: `#[tokio::test]` runs the test
    // body on a worker. Building a second runtime inside one panics, so hand
    // the future to the runtime that is already here.
    //
    // `block_in_place` is what makes that safe: it tells the scheduler this
    // worker is about to block, so the others keep running. It needs a
    // multi-threaded runtime, which is why the tests that reach this are
    // `#[tokio::test(flavor = "multi_thread")]`.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        return tokio::task::block_in_place(|| handle.block_on(future));
    }

    BRIDGE.with(|cell| {
        cell.get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("a current-thread runtime for synchronous store reads")
        })
        .block_on(future)
    })
}
