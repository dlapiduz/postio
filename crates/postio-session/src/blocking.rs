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
    /// **Multi-threaded, with one worker, and the flavour is the whole point.**
    /// `current_thread` is the obvious choice -- there is one caller, already
    /// on the thread it wants to be on, and a worker pool is a pool for
    /// nothing -- and it aborts the process the first time one of these reads
    /// nests inside another. A `current_thread` runtime refuses
    /// `block_in_place`, so the inner `now` below finds this runtime's handle,
    /// asks it to stand aside, and is told no.
    ///
    /// One worker rather than a pool: the future is driven on the calling
    /// thread by `block_on` either way, and the worker exists only so that
    /// `block_in_place` is a legal thing to ask for.
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
    // Already in a runtime's context. Three ways to get here, and only the
    // first was anticipated: a `#[tokio::test]` body on a worker; a callback
    // GTK fires while an outer `now` is running; and `now` inside `now`, which
    // is the ordinary shape of this application rather than an exotic one --
    // `onboarding.rs` answers *sign in* with one, and three frames down
    // `install_autosave` needs another.
    //
    // Building a second runtime inside one panics, so hand the future to the
    // runtime that is already here. `block_in_place` is what makes that safe:
    // it tells the scheduler this thread is about to block, so the rest of the
    // runtime keeps running. It is only legal on a multi-threaded one -- hence
    // the flavour of `BRIDGE`, and hence `#[tokio::test(flavor =
    // "multi_thread")]` on the tests that reach this.
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        // Said here rather than left to tokio, because tokio's own sentence --
        // *"can call blocking only when running on the multi-threaded
        // runtime"* -- is true, unactionable, and arrives through a GTK
        // trampoline that cannot unwind, so it comes with no backtrace worth
        // reading. The reader needs to know *which* runtime to go and change.
        assert!(
            handle.runtime_flavor() != tokio::runtime::RuntimeFlavor::CurrentThread,
            "a synchronous store read was reached from inside a current-thread \
             runtime, which can neither stand aside for `block_in_place` nor \
             answer a second `block_on`. Whichever runtime drives this \
             callback has to be built `new_multi_thread().worker_threads(1)` \
             -- see `postio_session::blocking`."
        );
        return tokio::task::block_in_place(|| handle.block_on(future));
    }

    BRIDGE.with(|cell| {
        cell.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .enable_all()
                .build()
                .expect("a runtime for synchronous store reads")
        })
        .block_on(future)
    })
}

#[cfg(test)]
mod tests {
    /// A GTK callback that reads the store, inside a GTK callback that reads
    /// the store — which is the ordinary case, not an exotic one.
    ///
    /// `onboarding.rs` answers *sign in* with `now(async { open_account(…) })`,
    /// `open_account` awaits `feed_the_window`, and `feed_the_window` calls
    /// `install_autosave`, which is synchronous and reads the store through
    /// `now` again. Every layer is behaving; the nesting is structural.
    ///
    /// It aborted the process on the first run against a real account. The
    /// outer call found no runtime and built the thread-local one, the inner
    /// call found *that* runtime's handle and asked it for `block_in_place` —
    /// which a `current_thread` runtime refuses, from a `#[no_unwind]` GTK
    /// trampoline, so the panic could not even unwind into a backtrace.
    #[test]
    fn a_store_read_inside_a_store_read_answers_rather_than_aborting() {
        let answer = super::now(async { super::now(async { 21 }) * 2 });
        assert_eq!(
            answer, 42,
            "the nested read came back wrong, which is a different bug from \
             the one this test is about"
        );
    }

    /// The same nesting from a worker of somebody else's runtime, which is
    /// where every `#[tokio::test(flavor = "multi_thread")]` caller starts.
    /// The one arrangement that cannot work, failing in words.
    ///
    /// A `current_thread` runtime can neither stand aside for
    /// `block_in_place` nor be asked for a second `block_on`, so a
    /// synchronous store read reached from inside one has no answer. What it
    /// must not do is what it did on 2026-09-13: abort the process from a GTK
    /// trampoline with tokio's own sentence, which says what is forbidden and
    /// not one word about which runtime to go and change.
    #[test]
    #[should_panic(expected = "worker_threads(1)")]
    fn a_current_thread_runtime_is_refused_by_name() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a runtime");
        runtime.block_on(async { super::now(async { 1 }) });
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn nesting_works_on_a_borrowed_runtime_too() {
        let answer = tokio::task::spawn_blocking(|| super::now(async { super::now(async { 7 }) }))
            .await
            .expect("the blocking task");
        assert_eq!(answer, 7);
    }
}
