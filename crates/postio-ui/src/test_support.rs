//! Counters the tests read, in `src/` because tests in several crates need them.
//!
//! The same idiom as [`postio_core::test_support`] and
//! `postio_storage::test_support::counting`, and the reasoning for counting
//! rather than timing is in [`crate::reader::cost`].
//!
//! Every reader here is a running total for the process. A test takes a
//! `before`, acts, and asserts on the delta -- which is what lets it measure
//! across an async boundary, where a scoped closure could not.

use std::sync::atomic::Ordering;

use crate::reader::cost;

/// How many reader documents this process has assembled.
pub fn documents_built() -> u64 {
    cost::DOCUMENTS.load(Ordering::Relaxed)
}

/// Total bytes of every document assembled.
pub fn document_bytes() -> u64 {
    cost::DOCUMENT_BYTES.load(Ordering::Relaxed)
}

/// The largest single document assembled.
///
/// The one that catches per-message bulk coming back: a mean stays
/// comfortable while one enormous document is handed over on every switch.
pub fn largest_document() -> u64 {
    cost::LARGEST_DOCUMENT.load(Ordering::Relaxed)
}

/// How many documents this process has handed to a rendering surface.
pub fn renders_issued() -> u64 {
    cost::RENDERS.load(Ordering::Relaxed)
}

/// How many rendering surfaces this process has created.
pub fn surfaces_created() -> u64 {
    cost::SURFACES_CREATED.load(Ordering::Relaxed)
}

/// Surfaces created and not yet released.
///
/// Signed, because a frontend that reports a release it never created is a
/// bug worth seeing rather than saturating quietly at zero.
pub fn surfaces_held() -> i64 {
    cost::SURFACES_CREATED.load(Ordering::Relaxed) as i64
        - cost::SURFACES_RELEASED.load(Ordering::Relaxed) as i64
}
