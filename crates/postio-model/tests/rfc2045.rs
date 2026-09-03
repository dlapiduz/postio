//! The MIME audit, made executable (#680).
//!
//! `docs/rfc-compliance.md` states the verdicts; this is the half a machine
//! can check, one case per row, so a verdict that stops being true fails a run
//! rather than becoming a wrong sentence in a file nobody re-reads. Same
//! arrangement as `rfc5322.rs`.
//!
//! **What is not here.** Two of the three gaps the audit found have no test
//! asserting the fix, because a test that asserts a bug is worthless and one
//! that asserts the fix cannot pass before it: #899 (a `--boundary` mid-line
//! truncates the body) and #901 (`encoding_problems` reaches nobody). Their
//! *inputs* are here and in the corpus, so the case each fix has to handle is
//! pinned even while the behaviour is wrong. #900 (an unusable boundary loses
//! the body and offers the container as an attachment) is fixed below.

use postio_model::{mime, test_corpus};

/// A `multipart/mixed` with one `text/plain` part carrying `body`.
fn multipart(body: &str) -> Vec<u8> {
    format!(
        "From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"SEP\"\r\n\r\n\
--SEP\r\nContent-Type: text/plain\r\n\r\n{body}\r\n--SEP--\r\n"
    )
    .into_bytes()
}

/// A single-part message with the given headers and body bytes.
fn single(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut raw =
        format!("From: ada@example.com\r\nMIME-Version: 1.0\r\n{headers}\r\n\r\n").into_bytes();
    raw.extend_from_slice(body);
    raw
}

// ---------------------------------------------------------------------------
// RFC 2045 §5 — Content-Type parameters
// ---------------------------------------------------------------------------

#[test]
fn an_rfc2231_filename_is_reassembled_across_continuations_and_decoded() {
    // The three spellings a non-ASCII filename arrives in are already a corpus
    // fixture; this is the one that needs both halves at once — continuation
    // numbering *and* a charset tag — because getting either wrong produces a
    // filename that looks plausible and is wrong.
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n\
--b\r\nContent-Type: text/plain\r\n\r\nhello\r\n\
--b\r\nContent-Type: application/pdf\r\n\
Content-Disposition: attachment;\r\n\tfilename*0*=utf-8''%C3%A9t%C3%A9;\r\n\
\tfilename*1*=%20report.pdf\r\n\r\nPDFBYTES\r\n--b--\r\n";
    let parsed = mime::parse(raw);
    assert_eq!(parsed.body.text.as_deref(), Some("hello"));
    assert_eq!(parsed.parts.len(), 1);
    assert_eq!(
        parsed.parts[0].attachment.filename.as_deref(),
        Some("été report.pdf")
    );
}

#[test]
fn a_quoted_boundary_may_contain_a_semicolon_and_a_space() {
    // The reason boundaries are allowed to be quoted at all, and the case a
    // parameter splitter that splits on `;` before it looks at quotes gets
    // wrong.
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"--=_Part 1; x\"\r\n\r\n\
----=_Part 1; x\r\nContent-Type: text/plain\r\n\r\nhello\r\n\
----=_Part 1; x--\r\n";
    assert_eq!(mime::parse(raw).body.text.as_deref(), Some("hello"));
}

#[test]
fn the_defaults_rfc2045_gives_are_filled_in() {
    // §5.2: no `Content-Type` means `text/plain; charset=us-ascii`, for the
    // message and for a part inside a multipart alike. A parser that treated
    // "absent" as "unknown" would put the body in the attachment list.
    assert_eq!(
        mime::parse(b"From: ada@example.com\r\n\r\nplain body\r\n")
            .body
            .text
            .as_deref(),
        Some("plain body\r\n")
    );
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\r\n--b\r\n\r\nno content type\r\n--b--\r\n";
    assert_eq!(
        mime::parse(raw).body.text.as_deref(),
        Some("no content type")
    );
}

// ---------------------------------------------------------------------------
// RFC 2045 §6 — Content-Transfer-Encoding
// ---------------------------------------------------------------------------

#[test]
fn quoted_printable_soft_breaks_join_and_undecodable_sequences_survive() {
    // §6.7. A soft break is a `=` at end of line and joins with nothing
    // between; `=ZZ` is not a hex pair and the RFC lets a robust decoder leave
    // it alone, which is what "show what arrived" means here.
    let parsed = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: quoted-printable",
        b"soft=\r\nbreak and a stray =ZZ\r\n",
    ));
    assert_eq!(
        parsed.body.text.as_deref(),
        Some("softbreak and a stray =ZZ\r\n")
    );
}

#[test]
fn base64_that_will_not_decode_degrades_to_its_own_text_and_says_so() {
    // The degradation is the right one -- raw text beats an empty body -- and
    // the flag is the only thing that could tell a person the words they are
    // reading are not the words that were sent. #901 is that the flag reaches
    // nobody; this pins the half that works.
    let parsed = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64",
        b"aGVsbG8g*d29ybGQ\r\n",
    ));
    assert_eq!(parsed.body.text.as_deref(), Some("aGVsbG8g*d29ybGQ\r\n"));
    assert!(
        parsed.encoding_problems,
        "an undecodable payload must at least be flagged"
    );
}

#[test]
fn base64_and_quoted_printable_round_trip_from_the_corpus() {
    for name in [
        "transfer-encoding-base64",
        "transfer-encoding-quoted-printable",
    ] {
        let parsed = mime::parse(test_corpus::load(name).bytes());
        assert!(
            parsed.body.text.is_some() || parsed.body.html.is_some(),
            "{name} decoded to no body at all"
        );
        assert!(
            !parsed.encoding_problems,
            "{name} is well-formed and must not be flagged"
        );
    }
}

#[test]
fn eight_bit_and_binary_bodies_arrive_as_their_own_bytes() {
    // §6.2 and §6.8: both mean "not encoded". Anything that tried to decode
    // them would corrupt exactly the mail that needed no help.
    let text = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit",
        "Grüße\r\n".as_bytes(),
    ));
    assert_eq!(text.body.text.as_deref(), Some("Grüße\r\n"));

    let binary = mime::parse(&single(
        "Content-Type: application/octet-stream\r\nContent-Transfer-Encoding: binary",
        b"\x00\x01\x02\r\n",
    ));
    assert_eq!(binary.parts.len(), 1);
    assert_eq!(binary.parts[0].content, b"\x00\x01\x02\r\n");
}

// ---------------------------------------------------------------------------
// RFC 2046 §5.1 — multipart structure
// ---------------------------------------------------------------------------

#[test]
fn a_delimiter_needs_its_own_line_and_may_carry_transport_padding() {
    // §5.1.1. The boundary word without its dashes is body text, and trailing
    // whitespace after a real delimiter is transport padding rather than part
    // of the boundary.
    assert_eq!(
        mime::parse(&multipart("one\r\nSEP\r\ntwo"))
            .body
            .text
            .as_deref(),
        Some("one\r\nSEP\r\ntwo"),
        "the boundary word without dashes is ordinary text"
    );
    assert_eq!(
        mime::parse(&multipart("one\r\n--SEP  \r\ntwo"))
            .body
            .text
            .as_deref(),
        Some("one"),
        "a delimiter's trailing whitespace is transport padding"
    );
}

#[test]
fn a_multipart_whose_boundary_never_appears_falls_back_to_its_own_text() {
    // #900, RFC 2046 §5.1.1: a multipart entity whose boundary parameter is
    // "unrecognisable" must be treated as `text/plain`. The header names a
    // boundary that was rewritten and the body still uses the old one, so
    // `mail_parser` cannot split it into parts at all -- the whole entity
    // comes back as a single part typed `multipart/alternative`, and the
    // fix is Postio's to read that raw content as the body rather than as a
    // nameless attachment (see the module docs above).
    let raw = test_corpus::load("multipart-boundary-never-appears").bytes();
    let parsed = mime::parse(raw);

    let text = parsed
        .body
        .text
        .expect("the fallback should have produced a body");
    assert!(
        text.contains("The body a person is supposed to read"),
        "the body text was not recovered: {text:?}"
    );
    assert!(
        parsed.parts.is_empty(),
        "the container must not be offered as an attachment: {:?}",
        parsed.parts
    );
}

#[test]
fn a_multipart_with_no_boundary_parameter_at_all_falls_back_to_its_own_text() {
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed\r\n\r\n--SEP\r\nContent-Type: text/plain\r\n\r\nthe body\r\n--SEP--\r\n";
    let parsed = mime::parse(raw);

    assert_eq!(
        parsed.body.text.as_deref(),
        Some("--SEP\r\nContent-Type: text/plain\r\n\r\nthe body\r\n--SEP--\r\n")
    );
    assert!(parsed.parts.is_empty(), "{:?}", parsed.parts);
}

#[test]
fn a_multipart_with_an_empty_boundary_parameter_falls_back_to_its_own_text() {
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=\"\"\r\n\r\n--SEP\r\nContent-Type: text/plain\r\n\r\nthe body\r\n--SEP--\r\n";
    let parsed = mime::parse(raw);

    assert_eq!(
        parsed.body.text.as_deref(),
        Some("--SEP\r\nContent-Type: text/plain\r\n\r\nthe body\r\n--SEP--\r\n")
    );
    assert!(parsed.parts.is_empty(), "{:?}", parsed.parts);
}

#[test]
fn a_multipart_fallback_body_is_not_flagged_as_flowed() {
    // `text_is_flowed` reads the part's own `format` attribute, and the
    // fallback part has none of the ones a real `text/plain` would -- it is
    // still the whole `multipart/*` header. False is the honest answer, not
    // a guess.
    let raw = test_corpus::load("multipart-boundary-never-appears").bytes();
    let parsed = mime::parse(raw);

    assert!(!parsed.text_is_flowed);
}

#[test]
fn an_ordinary_multipart_is_unaffected_by_the_fallback() {
    // The fallback triggers on the shape (multipart-typed, no children), not
    // on every multipart -- a message whose boundary actually works must
    // keep going through the ordinary path.
    let parsed = mime::parse(&multipart("hello"));

    assert_eq!(parsed.body.text.as_deref(), Some("hello"));
}

#[test]
fn the_lost_boundary_fixture_is_the_shape_the_fix_has_to_handle() {
    // The input half of #900. The body is right there in the bytes and the
    // boundary named in the header appears nowhere, which is what a gateway
    // that rewrote one and not the other leaves behind. Pinned so the fixture
    // cannot drift into being merely well-formed mail.
    let raw = test_corpus::load("multipart-boundary-never-appears").bytes();
    let text = String::from_utf8_lossy(raw);
    let (headers, body) = text.split_once("\r\n\r\n").expect("a header block");
    let declared = headers
        .split("boundary=\"")
        .nth(1)
        .and_then(|rest| rest.split('"').next())
        .expect("a boundary parameter");
    assert!(
        !body.contains(declared),
        "the fixture's boundary does appear after all, so it no longer tests anything"
    );
    assert!(
        body.contains("The body a person is supposed to read"),
        "the fixture must carry a body worth losing"
    );
}

#[test]
fn a_multipart_nested_forty_deep_neither_panics_nor_loses_its_leaf() {
    // The correctness half of the surface #147's fuzzer treats as adversarial.
    // `parse` is documented infallible; depth is the cheapest way to break
    // that promise, and the leaf still has to come back.
    let depth = 40;
    let mut raw = Vec::from(
        &b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b0\"\r\n\r\n"[..],
    );
    for n in 0..depth {
        raw.extend_from_slice(format!("--b{n}\r\n").as_bytes());
        if n + 1 < depth {
            raw.extend_from_slice(
                format!(
                    "Content-Type: multipart/mixed; boundary=\"b{}\"\r\n\r\n",
                    n + 1
                )
                .as_bytes(),
            );
        } else {
            raw.extend_from_slice(b"Content-Type: text/plain\r\n\r\ndeep\r\n");
        }
    }
    for n in (0..depth).rev() {
        raw.extend_from_slice(format!("--b{n}--\r\n").as_bytes());
    }
    assert_eq!(mime::parse(&raw).body.text.as_deref(), Some("deep"));
}

#[test]
fn the_preamble_and_the_epilogue_are_discarded() {
    // §5.1.1 again: the text before the first delimiter and after the closing
    // one is not part of any body. `multipart-alternative.eml` carries both
    // precisely so this cannot regress unnoticed.
    let parsed = mime::parse(test_corpus::load("multipart-alternative").bytes());
    let text = parsed.body.text.unwrap_or_default();
    let html = parsed.body.html.unwrap_or_default();
    for stray in ["multipart message in MIME format", "epilogue"] {
        assert!(
            !text.to_lowercase().contains(stray) && !html.to_lowercase().contains(stray),
            "{stray:?} leaked out of the preamble or epilogue"
        );
    }
}

// ---------------------------------------------------------------------------
// RFC 2046 §4.1.2 / RFC 2049 — charset declared versus actual bytes
// ---------------------------------------------------------------------------

#[test]
fn a_mislabelled_charset_degrades_to_readable_text_rather_than_to_an_error() {
    // The case that produces mojibake instead of a failure, and the one most
    // likely to be silently wrong. Every one of these has to come back as
    // *something* a person can look at: an empty reading pane is the one
    // answer that is worse than wrong characters, because it is
    // indistinguishable from a message that had nothing in it (#70).
    let utf8_as_ascii = single(
        "Content-Type: text/plain; charset=us-ascii",
        "été\r\n".as_bytes(),
    );
    let latin1_as_utf8 = single(
        "Content-Type: text/plain; charset=utf-8",
        &[0xe9, 0x74, 0xe9, b'\r', b'\n'],
    );
    let unheard_of = single(
        "Content-Type: text/plain; charset=x-not-a-charset",
        "Grüße\r\n".as_bytes(),
    );
    for (label, raw) in [
        ("utf-8 bytes labelled us-ascii", utf8_as_ascii),
        ("latin-1 bytes labelled utf-8", latin1_as_utf8),
        ("a charset nothing has heard of", unheard_of),
    ] {
        let text = mime::parse(&raw).body.text.unwrap_or_default();
        assert!(!text.trim().is_empty(), "{label} produced no body at all");
    }
}

#[test]
fn the_charset_corpus_still_produces_a_body_for_every_fixture() {
    for fixture in test_corpus::by_category(test_corpus::Category::NonUtf8Charset) {
        let parsed = mime::parse(fixture.bytes());
        assert!(
            parsed.body.text.is_some() || parsed.body.html.is_some(),
            "{} decoded to no body at all",
            fixture.name()
        );
    }
}
