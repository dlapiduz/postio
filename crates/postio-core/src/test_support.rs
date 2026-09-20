//! Counters the tests read, in `src/` because tests in several crates need them.
//!
//! The same idiom as `postio_storage::test_support::counting`: what gates a
//! performance claim here is a *count*, not a duration. A shared machine cannot
//! defend sixteen milliseconds, but "how many times did this happen" is the
//! same number everywhere, and it is the cause the duration is an effect of.

use std::sync::atomic::Ordering;

/// How many keymaps this process has resolved from the command registry.
///
/// Resolution is quadratic in the number of commands: every claim asks every
/// binding already made whether the key is taken, and each of those questions
/// goes back to the registry for the holder's contexts. Paying it once is
/// nothing; `postio-gtk`'s message row paid it per row GTK built, which was
/// most of the second a folder switch spent rebuilding the list (#1216).
///
/// [`Keymap::defaults`](crate::Keymap::defaults) is the answer for anything
/// that only wants the registry's own bindings, and this is what proves it is
/// actually resolved once.
pub fn keymap_resolutions() -> u64 {
    crate::config::RESOLUTIONS.load(Ordering::Relaxed)
}
