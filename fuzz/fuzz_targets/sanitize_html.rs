//! Bytes into the incoming sanitize path. Lossy UTF-8, not rejection: a real body arrives in
//! whatever charset the sender claimed.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    let html = String::from_utf8_lossy(raw);
    postio_fuzz::check_sanitize_html(&html);
});
