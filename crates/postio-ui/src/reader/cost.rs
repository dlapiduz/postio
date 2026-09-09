//! What moving between messages costs, counted rather than timed.
//!
//! The same idiom as [`postio_core::test_support::keymap_resolutions`] and
//! `postio_storage::test_support::counting`: what gates a performance claim
//! here is a *count*, not a duration. A shared machine cannot defend sixteen
//! milliseconds, but "how many times did this happen" is the same number
//! everywhere, and it is the cause the duration is an effect of.
//!
//! # Why the storage counter cannot answer this
//!
//! `postio_storage`'s counts come off SQLite's trace hook, and the defect
//! this module exists for issues **no query at all**: #749 found that
//! `Enter` on the row already under the cursor loaded the same message's
//! document a second time, because the filler was wired to both
//! `connect_cursor_moved` and `connect_activated` and only the first
//! deduplicates. Two identical documents, one store read, every existing test
//! green.
//!
//! # What is counted, and what each one catches
//!
//! * **documents built** and **bytes** — a document carrying bulk that is
//!   identical between messages. #749 measured ~1.2 MB of `@font-face` data
//!   URIs in *every* document, re-parsed by the engine on every switch; ADR
//!   0023 moved the bytes behind a scheme, and [`largest_document`] is what
//!   notices if anything like them comes back.
//! * **renders** — one gesture, at most one render, and none at all for
//!   re-selecting what is already displayed.
//! * **surfaces** — a rendering surface per message is what ADR 0032 measured
//!   at thirty processes for a thirty-message thread. Held is created minus
//!   released, so a conversation that never lets go shows up here even while
//!   every individual render looks cheap.
//!
//! Counters are never reset. Tests read a `before`, act, and assert on the
//! delta -- which is what lets them span an async boundary that a scoped
//! closure could not.
//!
//! # Per thread, not per process (#1390)
//!
//! They were `AtomicU64` statics, and a delta assertion over a shared counter
//! is only exact if nothing else is counting. libtest runs `#[test]`s on a
//! thread pool, so a test asserting "assembling a document counts one"
//! failed the moment another test in the same binary assembled one between
//! its snapshot and its assertion. The exactness is not the problem -- "a
//! frontend that grows a second path to the engine is invisible" is the
//! whole point of it -- so the counters moved instead.
//!
//! Everything that counts here runs on the thread that owns the interface:
//! documents are assembled on it, and a frontend's load choke point is on it
//! because the toolkit has no other. A counter incremented on a worker and
//! read on the main thread would read zero, which is worth knowing before
//! adding one -- there is no such caller today.

use std::cell::Cell;
use std::thread::LocalKey;

thread_local! {
    pub(crate) static DOCUMENTS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static DOCUMENT_BYTES: Cell<u64> = const { Cell::new(0) };
    pub(crate) static LARGEST_DOCUMENT: Cell<u64> = const { Cell::new(0) };
    pub(crate) static RENDERS: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SURFACES_CREATED: Cell<u64> = const { Cell::new(0) };
    pub(crate) static SURFACES_RELEASED: Cell<u64> = const { Cell::new(0) };
}

/// Add to a counter.
pub(crate) fn bump(counter: &'static LocalKey<Cell<u64>>, by: u64) {
    counter.with(|count| count.set(count.get().saturating_add(by)));
}

/// Raise a counter to `value` if it is higher — the high-water mark.
fn raise(counter: &'static LocalKey<Cell<u64>>, value: u64) {
    counter.with(|count| count.set(count.get().max(value)));
}

/// Read a counter.
pub(crate) fn read(counter: &'static LocalKey<Cell<u64>>) -> u64 {
    counter.with(Cell::get)
}

/// One document was assembled, of `bytes` bytes.
///
/// Called by the assembly itself rather than by a frontend, because the
/// question "did this document carry bulk" is answered where the document is
/// built and is the same answer for every frontend.
pub(crate) fn note_document(bytes: usize) {
    bump(&DOCUMENTS, 1);
    bump(&DOCUMENT_BYTES, bytes as u64);
    raise(&LARGEST_DOCUMENT, bytes as u64);
}

/// A document was handed to a rendering surface.
///
/// Called by a frontend at its single load choke point. Every frontend has
/// one; if a frontend grows a second path to the engine, this counter is what
/// makes that visible instead of silently doubling the cost of a keystroke.
pub fn note_render() {
    bump(&RENDERS, 1);
}

/// A rendering surface was created.
pub fn note_surface_created() {
    bump(&SURFACES_CREATED, 1);
}

/// A rendering surface was released.
///
/// Released rather than dropped, because what matters is whether the engine
/// process behind it can go, not whether a Rust value went out of scope.
pub fn note_surface_released() {
    bump(&SURFACES_RELEASED, 1);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A counter another thread is also using stays exact here (#1390).
    ///
    /// This is the failure that started it, reproduced deliberately: a delta
    /// assertion over a *shared* counter is only exact if nothing else is
    /// counting, and libtest runs tests on a thread pool. So
    /// `assembling_a_document_is_counted_with_its_size` failed with
    /// `left: 2, right: 1` the moment another test in the binary assembled a
    /// document between its snapshot and its assertion — which #1386's
    /// anchor test did, and which any future test that builds a document
    /// would do again.
    ///
    /// Against `AtomicU64` statics this fails. Against thread-local counters
    /// the other thread's thousand documents are its own, and this one still
    /// counts one.
    #[test]
    fn a_neighbours_counting_does_not_reach_this_thread() {
        let before = read(&DOCUMENTS);

        let busy = std::thread::spawn(|| {
            for _ in 0..1000 {
                note_document(512);
            }
            read(&DOCUMENTS)
        });
        note_document(16);
        let neighbour = busy.join().expect("the neighbour thread finished");

        assert_eq!(
            read(&DOCUMENTS) - before,
            1,
            "a thousand documents assembled beside this one were counted \
             against it, so every exact-delta assertion in this workspace is \
             a coin toss"
        );
        assert_eq!(
            neighbour, 1000,
            "the neighbour's own counting went somewhere else, which would \
             mean the counters are not per thread but merely broken"
        );
    }

    use crate::reader::document::{self, Sheet};
    use crate::test_support;
    use postio_body::RemoteImages;

    #[test]
    fn assembling_a_document_is_counted_with_its_size() {
        let before = test_support::documents_built();
        let bytes_before = test_support::document_bytes();

        let document = document::document_for("<p>hi</p>", RemoteImages::Blocked, Sheet::Theme);

        assert_eq!(
            test_support::documents_built() - before,
            1,
            "assembling a document has to be counted where it is assembled, or \
             a frontend that grows a second path to the engine is invisible"
        );
        assert_eq!(
            test_support::document_bytes() - bytes_before,
            document.len() as u64,
            "the bytes counted must be the bytes handed over -- an approximation \
             cannot catch per-message bulk coming back"
        );
        assert!(
            test_support::largest_document() >= document.len() as u64,
            "the largest document is what notices #749's inlined fonts returning"
        );
    }

    #[test]
    fn a_frontends_reports_reach_the_counters() {
        let renders = test_support::renders_issued();
        let created = test_support::surfaces_created();
        let held = test_support::surfaces_held();

        super::note_render();
        super::note_surface_created();
        super::note_surface_created();
        super::note_surface_released();

        assert_eq!(test_support::renders_issued() - renders, 1);
        assert_eq!(test_support::surfaces_created() - created, 2);
        assert_eq!(
            test_support::surfaces_held() - held,
            1,
            "held is created minus released: a conversation that never lets go \
             shows up here even while every individual render looks cheap"
        );
    }
}
