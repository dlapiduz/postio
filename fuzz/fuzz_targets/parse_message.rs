//! Raw bytes into the ingest path a fetched message runs through, seeded from
//! `crates/postio-model/tests/corpus/*.eml`.
//! Restores the panic hook: `fuzz_target!` aborts before unwinding, so a panic `mime::parse`
//! contains (#277) would look like a crash forever.
//! Uncaught panics are still crashes.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    postio_fuzz::allow_contained_panics();
    postio_fuzz::check_parse_message(raw);
});
