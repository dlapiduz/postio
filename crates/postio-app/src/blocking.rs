//! Answering a GTK callback that has to read the store, now.
//!
//! # Why this exists
//!
//! GTK callbacks are synchronous. A widget asks "what is this recipient's
//! name", "which mailbox is this row in", "what is the default signature" —
//! and needs the answer before it can lay out. There is nothing to hand a
//! future to.
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
pub(crate) fn now<T>(future: impl Future<Output = T>) -> T {
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
