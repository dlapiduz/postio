//! `mime::parse` is documented infallible, and #277 found that it was not.
//!
//! The module docs promise: "It never returns an error and never panics; a
//! message it cannot understand yields whatever could be recovered." A
//! malformed multipart broke the second half of that — `mail-parser` fires a
//! `debug_assert!` and the unwind went straight out of `parse`, on bytes that
//! arrive from the server during sync, before anyone opens anything.
//!
//! # What is and is not true about the severity
//!
//! Measured on 2026-08-25 against `mail-parser` 0.11.8, both ways:
//!
//! | | `debug_assertions` | `parse` |
//! |---|---|---|
//! | dev, test, CI, fuzz | on | **panics** |
//! | release (what ships) | off | returns, recovering nothing usable |
//!
//! The panic site is a `debug_assert!`, so the shipped binary never had this
//! crash — it silently recovers a message with no text, no html and no
//! subject, which the reader then labels "genuinely has no text or HTML
//! part". That is a wrong sentence rather than a denial of service, and
//! fixing it needs a signal `mail-parser` does not currently give in release;
//! it is not this test's business. What *is* this test's business is that
//! every build a contributor or CI runs used to crash on arrival, and the
//! crate's own documented contract was false.
//!
//! # The input
//!
//! Synthetic, from mutating the reserved-domain `.eml` corpus, then minimised
//! to 144 bytes — a truncated `Content-Type: multipart/` whose folded
//! `boundary=` value itself begins with `=`, wrapping `message/rfc822` parts
//! against a boundary that never properly opens. The flattened `A`s are the
//! minimiser reducing every byte it could. It is nobody's mail, and it is
//! inline rather than in the corpus because it is a crash reproducer, not a
//! message any loader should hand out as an example.

use postio_model::mime;

/// The #277 reproducer. See the module docs for where it came from.
const MALFORMED_MULTIPART: &[u8] =
    b"Content-Type:multipArt/\n\tboundAry==_report_e3a1\n\n--=_report_e3a1Content-Type:messAge/rfc822\nA\n\nA\nContent-Type:messAge/rfc822\n\n\n--=_report_e3a1--";

#[test]
fn a_malformed_multipart_does_not_panic() {
    // Pre-#277 this unwound out of `parse` in every build with debug
    // assertions on, which is every build the test suite runs in.
    let parsed = mime::parse(MALFORMED_MULTIPART);

    // The contract's other half: whatever could be recovered, and a size that
    // is still true, so the caller gets a row it can show.
    assert_eq!(parsed.size, MALFORMED_MULTIPART.len() as u64);
}

#[test]
fn a_malformed_multipart_does_not_panic_on_the_headers_only_path() {
    // Sync's first pass is headers-only, so it reaches a different arm of
    // `parse_inner` and needs its own containment.
    let parsed = mime::parse_headers(MALFORMED_MULTIPART);
    assert_eq!(parsed.size, MALFORMED_MULTIPART.len() as u64);
}

#[test]
fn ordinary_mail_still_parses_through_the_containment() {
    // The guard must not swallow success. A parse that works has to come back
    // whole, or containment would have turned a crash into silent data loss —
    // which is the trade the module docs exist to refuse.
    let raw = b"From: Ada Lovelace <ada@example.com>\r\n\
                To: Grace <grace@example.org>\r\n\
                Subject: Analytical engine notes\r\n\
                Message-ID: <note-1@example.com>\r\n\
                Content-Type: text/plain; charset=utf-8\r\n\
                \r\n\
                The engine weaves algebraic patterns.\r\n";

    let parsed = mime::parse(raw);
    assert_eq!(parsed.subject.as_deref(), Some("Analytical engine notes"));
    assert_eq!(parsed.from.len(), 1);
    assert_eq!(parsed.from[0].address, "ada@example.com");
    assert!(
        parsed
            .body
            .text
            .as_deref()
            .is_some_and(|text| text.contains("algebraic patterns")),
        "the body did not survive: {:?}",
        parsed.body.text
    );
}
