use postio_model::{mime, test_corpus};

fn multipart(body: &str) -> Vec<u8> {
    format!(
        "From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"SEP\"\r\n\r\n\
--SEP\r\nContent-Type: text/plain\r\n\r\n{body}\r\n--SEP--\r\n"
    )
    .into_bytes()
}

fn single(headers: &str, body: &[u8]) -> Vec<u8> {
    let mut raw =
        format!("From: ada@example.com\r\nMIME-Version: 1.0\r\n{headers}\r\n\r\n").into_bytes();
    raw.extend_from_slice(body);
    raw
}

#[test]
fn an_rfc2231_filename_is_reassembled_across_continuations_and_decoded() {
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
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"--=_Part 1; x\"\r\n\r\n\
----=_Part 1; x\r\nContent-Type: text/plain\r\n\r\nhello\r\n\
----=_Part 1; x--\r\n";
    assert_eq!(mime::parse(raw).body.text.as_deref(), Some("hello"));
}

#[test]
fn a_boundary_quoted_mid_line_does_not_end_the_part() {
    let parsed = mime::parse(&multipart("one\r\nnot --SEP here\r\ntwo"));
    assert_eq!(
        parsed.body.text.as_deref(),
        Some("one\r\nnot --SEP here\r\ntwo")
    );
}

#[test]
fn a_line_that_only_starts_with_the_boundary_does_not_end_the_part() {
    for body in [
        "FIRSTLINE\r\n--SEP this is not a delimiter\r\nSECONDLINE",
        "FIRSTLINE\r\n--SEP\tnor this\r\nSECONDLINE",
        "FIRSTLINE\r\n--SEP--this is no close either\r\nSECONDLINE",
        "ALPHA\r\n--SEPX\r\nBETA",
    ] {
        assert_eq!(
            mime::parse(&multipart(body)).body.text.as_deref(),
            Some(body),
            "the body was cut short"
        );
    }
}

#[test]
fn a_body_quoting_its_own_boundary_reads_whole_and_keeps_its_attachment() {
    let parsed = mime::parse(test_corpus::load("multipart-boundary-quoted-in-body").bytes());
    let text = parsed.body.text.expect("a text body");
    for line in [
        "1. Mid-line, as in: the parts are split on --=_quoted_5e0a here.",
        "--=_quoted_5e0a is only a delimiter when nothing but spaces follow it.",
        "--=_quoted_5e0aX is a different boundary altogether.",
        "All three are body text. This sentence is the end of the message.",
    ] {
        assert!(text.contains(line), "lost {line:?} from {text:?}");
    }
    assert!(!parsed.encoding_problems, "nothing here was damaged");
    let names: Vec<_> = parsed
        .parts
        .iter()
        .map(|part| part.attachment.filename.as_deref())
        .collect();
    assert_eq!(names, [Some("delimiter-rule.txt")]);
}

#[test]
fn the_defaults_rfc2045_gives_are_filled_in() {
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

#[test]
fn quoted_printable_soft_breaks_join_and_undecodable_sequences_survive() {
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
fn base64_with_a_stray_byte_is_read_around_it_and_says_so() {
    let parsed = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: base64",
        b"aGVsbG8g*d29ybGQ\r\n",
    ));
    assert_eq!(parsed.body.text.as_deref(), Some("hello world"));
    assert!(
        parsed.encoding_problems,
        "a damaged payload must at least be flagged"
    );
}

#[test]
fn an_unknown_transfer_encoding_is_flagged() {
    let parsed = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: x-uuencode",
        b"begin 644 note.txt\r\nM:&5L;&\\@=V]R;&0`\r\n`\r\nend\r\n",
    ));
    assert!(
        parsed.encoding_problems,
        "an encoding the parser does not know is a guess, and has to say so: \
         text={:?}",
        parsed.body.text
    );
}

#[test]
fn a_charset_that_decoded_lossily_is_flagged() {
    let parsed = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8",
        b"\xe9t\xe9 was the summer\r\n",
    ));
    assert!(
        parsed
            .body
            .text
            .as_deref()
            .is_some_and(|text| text.contains('\u{fffd}')),
        "the fixture is meant to decode lossily: {:?}",
        parsed.body.text
    );
    assert!(
        parsed.encoding_problems,
        "a body that lost octets to U+FFFD is exactly the mojibake the flag's \
         own doc comment promises to carry"
    );
}

#[test]
fn a_charset_that_decoded_cleanly_is_not_flagged() {
    let parsed = mime::parse(&single(
        "Content-Type: text/plain; charset=utf-8",
        "the glyph \u{fffd} is what a font shows for a missing character\r\n".as_bytes(),
    ));
    assert!(
        !parsed.encoding_problems,
        "a well-formed utf-8 body was flagged because of its own contents"
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

#[test]
fn a_delimiter_needs_its_own_line_and_may_carry_transport_padding() {
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
    let raw = test_corpus::load("multipart-boundary-never-appears").bytes();
    let parsed = mime::parse(raw);

    assert!(!parsed.text_is_flowed);
}

#[test]
fn an_ordinary_multipart_is_unaffected_by_the_fallback() {
    let parsed = mime::parse(&multipart("hello"));

    assert_eq!(parsed.body.text.as_deref(), Some("hello"));
}

#[test]
fn the_lost_boundary_fixture_is_the_shape_the_fix_has_to_handle() {
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

#[test]
fn a_mislabelled_charset_degrades_to_readable_text_rather_than_to_an_error() {
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

#[test]
fn a_text_html_part_whose_quoted_printable_is_invalid_is_still_shown() {
    let raw = single(
        "Content-Type: text/html; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: quoted-printable",
        b"<p>caf=C3=A9</p><img src=\"data:image/png;base64,AA==\">\r\n",
    );
    let message = mime::parse(&raw);
    let html = message
        .body
        .html
        .as_deref()
        .expect("the html body is shown");
    assert!(
        html.contains("<p>café</p>"),
        "the valid encoding is still decoded: {html:?}"
    );
    assert!(
        html.contains("AA==\""),
        "the sender's `==` is kept as written: {html:?}"
    );
    assert!(
        message.encoding_problems,
        "the caveat still stands: the words on screen are a lenient reading"
    );
    assert!(
        message.parts.is_empty(),
        "the body is not also offered as an attachment: {:?}",
        message.parts
    );
}

#[test]
fn a_text_plain_part_whose_quoted_printable_is_invalid_is_decoded_leniently() {
    let raw = single(
        "Content-Type: text/plain; charset=\"utf-8\"\r\n\
Content-Transfer-Encoding: quoted-printable",
        b"50=25 off==\r\ncaf=C3=A9 =\r\nsoft\r\n",
    );
    let message = mime::parse(&raw);
    let text = message
        .body
        .text
        .as_deref()
        .expect("the text body is shown");
    assert_eq!(text, "50% off==\r\ncafé soft\r\n");
    assert!(message.encoding_problems);
}
