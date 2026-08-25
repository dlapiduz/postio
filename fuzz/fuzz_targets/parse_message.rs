//! Raw bytes → the ingest path a fetched message runs through.
//!
//! Seeded from `crates/postio-model/tests/corpus/*.eml`, which is the best
//! starting corpus this project has: real messages, already covering charsets,
//! nested MIME, RFC 2047 words and broken `References` chains. libFuzzer takes
//! those apart from there.
//!
//! The judgement is in `postio_fuzz::check_parse_message`; see that crate's
//! docs for why it is not in here.
//!
//! # Why this one restores the panic hook
//!
//! This target found #277: `mail-parser` panics on a malformed multipart and
//! the unwind came straight out of `mime::parse`, which is documented as
//! infallible. That is fixed — `parse` contains it now, and in the shipping
//! application such a message reads as one with no body rather than taking
//! sync down.
//!
//! A fuzz target cannot see that on its own. `fuzz_target!` installs a panic
//! hook that aborts before unwinding, so `catch_unwind` never runs and a
//! contained panic still looks like a crash. Without
//! [`postio_fuzz::allow_contained_panics`] this target would report a fixed
//! bug forever, which is the same uselessness as a target that ignores a real
//! one. Uncaught panics are still crashes — see that function's docs.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    postio_fuzz::allow_contained_panics();
    postio_fuzz::check_parse_message(raw);
});
