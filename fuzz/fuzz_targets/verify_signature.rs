//! A sender's bytes into the signature check, which runs on every signed message that arrives.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    postio_fuzz::allow_contained_panics();
    postio_fuzz::check_verify_signature(raw);
});
