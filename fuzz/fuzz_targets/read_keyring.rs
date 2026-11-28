//! Bytes into both GnuPG keyring readers: GnuPG may be writing the file while Postio reads it.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    postio_fuzz::allow_contained_panics();
    postio_fuzz::check_read_keyring(raw);
});
