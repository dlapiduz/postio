use postio_model::mime;

const MALFORMED_MULTIPART: &[u8] =
    b"Content-Type:multipArt/\n\tboundAry==_report_e3a1\n\n--=_report_e3a1Content-Type:messAge/rfc822\nA\n\nA\nContent-Type:messAge/rfc822\n\n\n--=_report_e3a1--";

#[test]
fn a_malformed_multipart_does_not_panic() {
    let parsed = mime::parse(MALFORMED_MULTIPART);

    assert_eq!(parsed.size, MALFORMED_MULTIPART.len() as u64);
}

#[test]
fn a_malformed_multipart_does_not_panic_on_the_headers_only_path() {
    let parsed = mime::parse_headers(MALFORMED_MULTIPART);
    assert_eq!(parsed.size, MALFORMED_MULTIPART.len() as u64);
}

#[test]
fn ordinary_mail_still_parses_through_the_containment() {
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
