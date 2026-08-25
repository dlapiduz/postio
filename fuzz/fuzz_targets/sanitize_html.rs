//! Bytes → `postio_body`'s incoming sanitize path.
//!
//! Lossy UTF-8 rather than rejecting non-UTF-8 input: a real message body
//! arrives as bytes in whatever charset the sender claimed, and the decode
//! ahead of this is exactly the sort of thing that produces surprising
//! `String`s. Rejecting them here would hide that.
//!
//! The judgement is in `postio_fuzz::check_sanitize_html`.

#![no_main]

libfuzzer_sys::fuzz_target!(|raw: &[u8]| {
    let html = String::from_utf8_lossy(raw);
    postio_fuzz::check_sanitize_html(&html);
});
