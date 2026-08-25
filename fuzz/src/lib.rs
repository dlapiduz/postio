//! What the fuzz targets actually assert.
//!
//! A `fuzz_target!` body is unusually hard to test: it only runs under
//! libFuzzer, and the way you find out it was wrong is that it never fails.
//! So the judgement lives here as ordinary functions with ordinary tests, and
//! each target is a two-line wrapper that decodes its input and calls one.
//!
//! Two kinds of check, and the difference matters when triaging a find:
//!
//! * **Crash-freedom.** Simply calling the function is the assertion — a
//!   panic, an abort or an OOM is the find. This is most of what
//!   [`check_parse_message`] does, because a mail that crashes the client on
//!   sight is a denial of service the user re-triggers on every startup, and
//!   there is nothing else to say about a header block that parses.
//! * **Properties.** Something that must be true of the *output*, checked
//!   explicitly. These catch the failures that do not crash: a sanitizer that
//!   lets a scheme through, a span that points into the middle of a character.
//!   A property find is usually the more serious of the two.
//!
//! Every property here is written so that violating it is a bug in Postio,
//! never in the input. There is no such thing as a malformed input to any of
//! these functions — see each `check_*` for what "total" means for it.

pub mod properties;

/// Let a *contained* panic be contained, instead of aborting the process.
///
/// `libfuzzer_sys::fuzz_target!` installs a panic hook that calls
/// `process::abort()`, deliberately: aborting before the stack unwinds is what
/// lets libFuzzer tell one crash apart from another. The side effect is that
/// `std::panic::catch_unwind` never gets to run inside a fuzz target — the
/// hook fires first, and the process is gone.
///
/// That makes a target blind to exactly the thing #277 added.
/// `postio_model::mime::parse` contains a `mail-parser` panic on purpose, so
/// in the shipping application a malformed multipart is a message with no
/// body; under the default fuzz hook it is still a crash, and `parse_message`
/// would report a fixed bug forever.
///
/// So this replaces the hook with one that does nothing and lets the unwind
/// proceed. **An uncaught panic is still a crash**: it unwinds out of the
/// target closure and into libFuzzer's `extern "C"` frame, and Rust aborts
/// rather than unwind across that boundary. Assertion failures in
/// [`properties`] are therefore still found —
/// `an_uncaught_panic_still_aborts` in the fuzz workflow's own run is the
/// check on that, and the `should_panic` tests beside each property prove the
/// assertions fire.
///
/// Idempotent, and only for the target that needs it.
pub fn allow_contained_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Silent: a contained parse failure is an expected outcome here, and
        // one line of dependency panic text per malformed input would bury
        // libFuzzer's own output at several hundred execs a second.
        std::panic::set_hook(Box::new(|_| {}));
    });
}

pub use properties::{check_parse_message, check_parse_query, check_sanitize_html};
