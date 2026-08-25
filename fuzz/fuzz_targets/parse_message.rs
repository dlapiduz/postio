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
//! # This target is known red
//!
//! It finds #277 within a few minutes: `mail-parser` panics on a malformed
//! multipart, and the panic comes straight out of `mime::parse`, which is
//! documented as infallible. That is a real remotely-triggerable crash, not a
//! flaw in this target, and it is deliberately not worked around here — a
//! target taught to ignore a genuine find is worth less than no target. If
//! this stops at "Invalid part ID, could not find multipart", that is #277 and
//! not something you just broke.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    postio_fuzz::check_parse_message(raw);
});
