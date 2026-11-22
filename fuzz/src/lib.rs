//! What the fuzz targets assert, as ordinary tested functions; each target is a two-line wrapper.
//! Two kinds of check: crash-freedom (calling it is the assertion) and properties of the output,
//! which catch failures that do not crash.
//! Every property is written so a violation is a bug in Postio, never in the input.

pub mod properties;

/// Let a contained panic be contained instead of aborting the process.
///
/// `fuzz_target!` installs a hook that aborts before unwinding, so `catch_unwind` never runs and a
/// panic `postio_model::mime::parse` contains on purpose (#277) would look like a crash. This
/// replaces the hook with a no-op. An uncaught panic still aborts at libFuzzer's `extern "C"`
/// frame, so assertion failures are still found.
pub fn allow_contained_panics() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        // Silent: one line of dependency panic text per malformed input would bury libFuzzer's
        // output.
        std::panic::set_hook(Box::new(|_| {}));
    });
}

pub use properties::{check_parse_message, check_parse_query, check_sanitize_html};
