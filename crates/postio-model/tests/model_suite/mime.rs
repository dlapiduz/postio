use chrono::{DateTime, TimeZone, Utc};

use postio_model::mime::{self, ParsedMessage};
use postio_model::test_corpus::{self, Category};
use postio_model::{AccountId, BodyState, Disposition, MailboxId, RfcMessageId};

fn parse(name: &str) -> ParsedMessage {
    mime::parse(test_corpus::load(name).bytes())
}

fn addresses(list: &[postio_model::EmailAddress]) -> Vec<String> {
    list.iter().map(|address| address.address.clone()).collect()
}

#[test]
fn every_fixture_in_the_corpus_parses_without_panicking() {
    for fixture in test_corpus::all() {
        let parsed = mime::parse(fixture.bytes());

        assert_eq!(
            parsed.size,
            fixture.len() as u64,
            "{}: size is the raw byte count",
            fixture.name()
        );
    }
}

#[test]
fn every_fixture_parses_to_headers_only_without_panicking() {
    for fixture in test_corpus::all() {
        let parsed = mime::parse_headers(fixture.bytes());

        assert_eq!(
            parsed.body_state,
            BodyState::HeadersOnly,
            "{}: a header-only parse says so",
            fixture.name()
        );
        assert!(
            parsed.body.is_empty() && parsed.parts.is_empty(),
            "{}: a header-only parse buffers no body and no parts",
            fixture.name()
        );
    }
}

#[test]
fn a_message_with_no_headers_at_all_still_parses() {
    let parsed = mime::parse(b"this is not a message");

    assert!(parsed.from.is_empty());
    assert!(parsed.subject.is_none());
    assert_eq!(parsed.size, 21);
}

#[test]
fn an_empty_input_parses_to_an_empty_message() {
    let parsed = mime::parse(b"");

    assert_eq!(parsed.size, 0);
    assert!(parsed.headers.is_empty());
    assert!(parsed.body.is_empty());
}

#[test]
fn the_simplest_message_maps_onto_the_model() {
    let parsed = parse("plain-text-simple");

    assert_eq!(
        parsed.rfc_message_id,
        Some(RfcMessageId::new(
            "<20260210T091500.plain.simple@example.com>"
        ))
    );
    assert_eq!(parsed.subject.as_deref(), Some("Tuesday walkthrough notes"));
    assert_eq!(
        parsed.date,
        Some(Utc.with_ymd_and_hms(2026, 2, 10, 9, 15, 0).unwrap())
    );
    assert_eq!(addresses(&parsed.from), ["ada.norwood@example.com"]);
    assert_eq!(
        parsed.from[0].name.as_deref(),
        Some("Ada Norwood"),
        "the display name comes across too"
    );
    assert_eq!(addresses(&parsed.to), ["quinn.abara@example.net"]);
    assert!(parsed.cc.is_empty() && parsed.bcc.is_empty());

    let text = parsed.body.text.as_deref().expect("a text body");
    assert!(text.contains("The gate timings on the north entrance."));
    assert!(parsed.body.html.is_none(), "there is no HTML part");
    assert_eq!(parsed.body_state, BodyState::Full);

    let preview = parsed.preview.as_deref().expect("a preview");
    assert!(preview.starts_with("Quinn,"), "got: {preview:?}");
    assert!(
        preview.chars().count() <= mime::PREVIEW_CHARS,
        "the preview is bounded"
    );
    assert!(!preview.contains('\r'), "and is a single flat run of text");
}

#[test]
fn the_full_header_block_is_preserved_in_order_with_duplicates() {
    let parsed = parse("header-folding-received-chain");

    let received = parsed.headers.get_all("Received");
    assert_eq!(
        received.len(),
        3,
        "every hop of the Received chain is kept, in wire order"
    );
    assert!(
        received[0].contains("by imap.example.net"),
        "the last hop is first, as it appears on the wire: {:?}",
        received[0]
    );
    assert!(
        !received[0].contains('\n'),
        "folded headers are unfolded into one line: {:?}",
        received[0]
    );
    assert!(parsed.headers.contains("DKIM-Signature"));
    assert!(
        parsed.headers.get("subject").is_some(),
        "header lookup is case-insensitive"
    );
}

#[test]
fn addresses_are_normalized_and_groups_are_flattened() {
    let parsed = mime::parse(
        b"From: Ada Norwood <Ada.Norwood@Example.COM>\r\n\
          To: Team: quinn@example.net, jo@example.org;\r\n\
          Cc: <  spaced@example.com  >\r\n\
          Subject: Groups and spacing\r\n\r\nbody\r\n",
    );

    assert_eq!(addresses(&parsed.from), ["Ada.Norwood@Example.COM"]);
    assert_eq!(
        parsed.from[0].normalized(),
        "ada.norwood@example.com",
        "case is preserved verbatim and folded only on demand"
    );
    assert_eq!(
        addresses(&parsed.to),
        ["quinn@example.net", "jo@example.org"],
        "a group's members are flattened into the recipient list"
    );
    assert_eq!(
        addresses(&parsed.cc),
        ["spaced@example.com"],
        "surrounding whitespace is trimmed"
    );
}

#[test]
fn an_address_header_with_no_addr_spec_contributes_nothing() {
    let parsed = mime::parse(b"From: Nobody\r\nTo: \r\nSubject: x\r\n\r\nbody\r\n");

    assert!(
        parsed.from.is_empty(),
        "a display name with no address is not an address"
    );
    assert!(parsed.to.is_empty());
}

#[test]
fn iso_8859_1_bodies_and_encoded_words_are_decoded() {
    let parsed = parse("charset-iso-8859-1");

    assert_eq!(
        parsed.subject.as_deref(),
        Some("Gruß aus München – Angebot für März")
    );
    assert_eq!(parsed.from[0].name.as_deref(), Some("Jürgen Möller"));
    let text = parsed.body.text.as_deref().expect("a text body");
    assert!(text.contains("für März"), "got: {text:?}");
}

#[test]
fn shift_jis_in_base64_is_decoded_in_both_headers_and_body() {
    let parsed = parse("charset-shift-jis");

    let subject = parsed.subject.as_deref().expect("a subject");
    assert!(
        subject
            .chars()
            .any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)),
        "the subject decodes to Japanese, not mojibake: {subject:?}"
    );
    let text = parsed.body.text.as_deref().expect("a text body");
    assert!(
        text.chars().any(|c| ('\u{3000}'..='\u{9fff}').contains(&c)),
        "and so does the base64 Shift_JIS body: {text:?}"
    );
    assert!(!text.contains('\u{fffd}'), "with no replacement characters");
}

#[test]
fn windows_1252_bytes_labelled_latin_1_still_decode_to_the_right_glyphs() {
    let parsed = parse("charset-windows-1252-mislabeled");
    let text = parsed.body.text.as_deref().expect("a text body");

    assert!(
        text.contains('\u{201c}') && text.contains('\u{2014}'),
        "the C1 bytes are read as windows-1252 curly quotes and an em dash, \
         which is what every real client does: {text:?}"
    );
}

#[test]
fn utf_7_is_decoded_where_it_is_declared() {
    let parsed = parse("charset-utf-7");

    let subject = parsed.subject.as_deref().expect("a subject");
    assert!(
        subject.contains("Møller"),
        "the UTF-7 encoded word decodes: {subject:?}"
    );
}

#[test]
fn utf_8_bodies_survive_emoji_rtl_and_combining_marks() {
    let parsed = parse("charset-utf-8-emoji-rtl");
    let text = parsed.body.text.as_deref().expect("a text body");

    assert!(!text.contains('\u{fffd}'), "nothing was mangled");
    assert!(
        text.chars().any(|c| c as u32 > 0xffff),
        "astral-plane glyphs come through: {text:?}"
    );
}

#[test]
fn base64_and_quoted_printable_bodies_are_decoded() {
    let base64 = parse("transfer-encoding-base64");
    let text = base64.body.text.as_deref().expect("a text body");
    assert!(
        !text.contains("=="),
        "the body is decoded, not left as base64: {text:?}"
    );
    assert!(text.contains(' '), "and reads as prose");

    let quoted = parse("transfer-encoding-quoted-printable");
    let text = quoted.body.text.as_deref().expect("a text body");
    assert!(
        text.contains("an equals sign (=) that must survive as =3D"),
        "=3D decodes to a literal `=`, and `=3D3D` to a literal `=3D`: {text:?}"
    );
    assert!(
        text.contains("something to chew on"),
        "a soft line break is removed rather than turned into a newline: {text:?}"
    );
    assert!(text.contains("Ångström"), "and the accented run: {text:?}");
}

#[test]
fn adjacent_encoded_words_join_without_a_space() {
    let parsed = parse("encoded-word-subject-and-names");

    let subject = parsed.subject.as_deref().expect("a subject");
    assert!(
        subject.starts_with("Réunion du comité:"),
        "two charsets in one folded field: {subject:?}"
    );
    assert!(subject.contains("(détails ci-dessous)"), "got: {subject:?}");

    let names: Vec<_> = parsed
        .to
        .iter()
        .map(|address| address.name.clone().unwrap_or_default())
        .collect();
    assert!(
        names.contains(&"Åsa Österlund".to_owned()),
        "got: {names:?}"
    );
    assert!(
        names.contains(&"José Martínez".to_owned()),
        "got: {names:?}"
    );
    assert_eq!(
        parsed.cc[0].name.as_deref(),
        Some("田中 陽子"),
        "adjacent B-encoded words join with no space inserted between them"
    );
}

#[test]
fn broken_encoded_words_degrade_instead_of_panicking() {
    let parsed = parse("encoded-word-broken");

    assert!(
        parsed.subject.is_some(),
        "an unterminated or invalid encoded word still yields a subject"
    );
    assert!(parsed.headers.len() > 3, "and the header block survives");
}

#[test]
fn a_utf_8_encoded_word_decodes_outside_a_full_message() {
    assert_eq!(
        mime::decode_header_text(b"=?UTF-8?B?Q2Fmw6k=?="),
        "Café",
        "the same base64 UTF-8 word `parse` would decode in a Subject header"
    );
}

#[test]
fn a_non_utf_8_charset_decodes_rather_than_being_lossy_replaced() {
    assert_eq!(
        mime::decode_header_text(b"=?ISO-8859-1?B?Q2Fm6Q==?="),
        "Café",
        "the charset the word declares, not a UTF-8 guess over its raw bytes"
    );
}

#[test]
fn plain_ascii_with_no_encoded_word_passes_through() {
    assert_eq!(
        mime::decode_header_text(b"Shopping budget update"),
        "Shopping budget update"
    );
}

#[test]
fn an_empty_field_decodes_to_an_empty_string() {
    assert_eq!(mime::decode_header_text(b""), "");
}

#[test]
fn a_truncated_encoded_word_degrades_to_something_readable() {
    let text = mime::decode_header_text(b"=?UTF-8?B?Q2Fmw6k Shopping");
    assert!(!text.is_empty(), "got: {text:?}");
    assert!(text.contains("Shopping"), "got: {text:?}");
}

#[test]
fn decode_header_text_agrees_with_a_full_parse_of_the_same_subject() {
    let encoded = "=?UTF-8?B?Q2Fmw6k=?= budget update";
    let from_envelope = mime::decode_header_text(encoded.as_bytes());

    let raw = format!("Subject: {encoded}\r\n\r\nBody\r\n");
    let from_full_parse = mime::parse(raw.as_bytes());

    assert_eq!(
        from_full_parse.subject.as_deref(),
        Some(from_envelope.as_str()),
        "the subject the list shows before the body arrives must equal the one it shows after"
    );
}

fn breaks_the_line(c: char) -> bool {
    c.is_control() || c == '\u{2028}' || c == '\u{2029}'
}

fn assert_one_line(what: &str, text: &str) {
    assert!(
        !text.chars().any(breaks_the_line),
        "{what} carries a line break or control character, which draws as \
         a second line of chrome or not at all: {text:?}"
    );
}

#[test]
fn an_encoded_line_break_in_a_subject_folds_to_one_space() {
    for parsed in [
        parse("encoded-word-crlf-in-header"),
        mime::parse_headers(test_corpus::load("encoded-word-crlf-in-header").bytes()),
    ] {
        assert_eq!(
            parsed.subject.as_deref(),
            Some("Invoice attached Bcc: mallory@sender.invalid"),
            "the forged Bcc line has to read as part of the subject it is"
        );
        let name = parsed.from[0].name.as_deref().unwrap_or_default();
        assert_eq!(name, "Mallory X-Injected-By-Name: yes");
    }
}

#[test]
fn control_characters_in_a_decoded_subject_or_name_fold_to_one_space() {
    for parsed in [
        parse("encoded-word-control-chars-in-header"),
        mime::parse_headers(test_corpus::load("encoded-word-control-chars-in-header").bytes()),
    ] {
        let subject = parsed.subject.as_deref().expect("a subject");
        assert_one_line("the subject", subject);
        assert_eq!(
            subject,
            "Payment overdue From: Accounts <accounts@example.com> Date: today [31m urgent [0m end",
            "each run of breaks and controls is one space, and nothing else moves"
        );
        let name = parsed.from[0].name.as_deref().expect("a display name");
        assert_one_line("the display name", name);
        assert_eq!(name, "Billing Reply-To: refunds@sender.invalid");
        assert_eq!(parsed.from[0].address, "billing@sender.invalid");
    }
}

#[test]
fn every_fixture_s_subject_and_names_are_one_line() {
    for fixture in test_corpus::all() {
        let parsed = mime::parse(fixture.bytes());
        if let Some(subject) = parsed.subject.as_deref() {
            assert_one_line(&format!("{}'s subject", fixture.name()), subject);
        }
        for address in parsed.from.iter().chain(&parsed.to).chain(&parsed.cc) {
            if let Some(name) = address.name.as_deref() {
                assert_one_line(&format!("a name in {}", fixture.name()), name);
            }
        }
    }
}

#[test]
fn an_envelope_subject_folds_the_same_way_a_full_parse_does() {
    let encoded = "=?utf-8?Q?Invoice_attached=0D=0ABcc:_mallory=40sender=2Einvalid?=";
    assert_eq!(
        mime::decode_header_text(encoded.as_bytes()),
        "Invoice attached Bcc: mallory@sender.invalid"
    );
    let controls = "=?utf-8?Q?a=1B[1mb=00=07c=E2=80=A9d=C2=85e=09f?=";
    assert_eq!(
        mime::decode_header_text(controls.as_bytes()),
        "a [1mb c d e f"
    );
}

#[test]
fn multipart_alternative_keeps_both_forms_and_drops_the_preamble() {
    let parsed = parse("multipart-alternative");

    let text = parsed.body.text.as_deref().expect("a text part");
    let html = parsed.body.html.as_deref().expect("an html part");
    assert!(text.contains("The plain part says the meeting moved to 11:00"));
    assert!(html.contains("<strong>HTML part</strong>"));
    assert!(
        !text.contains("multi-part message in MIME format"),
        "the preamble is not body text: {text:?}"
    );
    assert!(
        !text.contains("epilogue") && !html.contains("epilogue"),
        "and neither is the epilogue"
    );
    assert!(
        parsed.attachments().next().is_none(),
        "an alternative pair is not an attachment"
    );
}

#[test]
fn a_format_flowed_text_part_is_recorded_as_such() {
    let parsed = parse("plain-text-flowed-reply");
    assert!(
        parsed.text_is_flowed,
        "the fixture's Content-Type declares format=flowed"
    );
}

#[test]
fn an_ordinary_text_part_with_no_format_parameter_is_not_flowed() {
    let parsed = parse("plain-text-simple");
    assert!(
        !parsed.text_is_flowed,
        "nothing here declared format=flowed, so short lines must stay short lines"
    );
}

#[test]
fn text_is_flowed_survives_into_message() {
    let message = parse("plain-text-flowed-reply").into_message(
        AccountId::new(1),
        MailboxId::new(1),
        received_at(),
    );
    assert!(
        message.text_is_flowed,
        "into_message must not drop the flag `ParsedMessage` recorded"
    );
}

#[test]
fn a_newsletter_has_a_body_and_no_attachments() {
    let parsed = parse("html-newsletter");

    assert!(parsed.body.html.is_some(), "the HTML part is the message");
    assert!(
        parsed.attachments().next().is_none(),
        "nothing in a newsletter is an attachment"
    );
    assert!(
        parsed.headers.contains("List-Unsubscribe"),
        "and its list headers are preserved for later"
    );
}

#[test]
fn attachments_carry_filename_type_size_and_disposition() {
    let parsed = parse("attachment-pdf");

    let attachments: Vec<_> = parsed.attachments().collect();
    assert_eq!(attachments.len(), 2, "two attachments, one of them a PDF");

    let pdf = attachments
        .iter()
        .find(|attachment| attachment.mime_type == "application/pdf")
        .expect("the PDF");
    assert_eq!(pdf.filename.as_deref(), Some("layout-rev-c.pdf"));
    assert_eq!(pdf.disposition, Disposition::Attachment);
    assert!(pdf.size > 0, "the size is the decoded byte count");
    assert!(pdf.content_id.is_none());
    assert!(
        pdf.blob_id.is_none(),
        "parsing stores no bytes; that is the blob store's job"
    );

    let checksums = attachments
        .iter()
        .find(|attachment| attachment.filename.as_deref() == Some("checksums.txt"))
        .expect("the text attachment");
    assert_eq!(
        checksums.disposition,
        Disposition::Attachment,
        "a text/plain part marked as an attachment is not a body"
    );
    assert!(
        parsed
            .body
            .text
            .as_deref()
            .is_some_and(|text| !text.contains("sha256")),
        "and its content does not leak into the body"
    );
}

#[test]
fn a_parts_decoded_bytes_are_available_and_match_its_declared_size() {
    let parsed = parse("attachment-pdf");

    let pdf = parsed
        .parts
        .iter()
        .find(|part| part.attachment.mime_type == "application/pdf")
        .expect("the PDF part");
    assert_eq!(pdf.content.len() as u64, pdf.attachment.size);
    assert!(
        pdf.content.starts_with(b"%PDF-"),
        "the base64 was decoded back to the bytes the sender sent"
    );
}

#[test]
fn rfc2231_and_encoded_word_filenames_are_decoded() {
    let parsed = parse("attachment-rfc2231-filename");

    let filenames: Vec<String> = parsed
        .attachments()
        .filter_map(|attachment| attachment.filename.clone())
        .collect();

    assert!(
        filenames
            .iter()
            .any(|name| name.starts_with("Ångström site survey")),
        "RFC 2231 continuations are joined and percent-decoded: {filenames:?}"
    );
    assert!(
        filenames.iter().any(|name| name == "räkneskap.txt"),
        "charset'language form: {filenames:?}"
    );
    assert!(
        filenames.iter().any(|name| name == "månadsrapport.txt"),
        "an encoded word inside a filename parameter, which is not legal but \
         is widely emitted: {filenames:?}"
    );
}

#[test]
fn a_body_part_disposed_inline_is_still_the_body() {
    let parsed = parse("inline-disposed-body");

    let html = parsed
        .body
        .html
        .as_deref()
        .expect("the text/html alternative is the body, however it is disposed");
    assert!(html.contains("<strong>confirmed</strong>"));
    assert!(
        parsed
            .body
            .text
            .as_deref()
            .is_some_and(|text| text.contains("confirmed")),
        "and the plain-text alternative is still the plain-text body"
    );
    assert_eq!(
        parsed.attachments().count(),
        0,
        "a message's own words are not attachments, so nothing here gets a chip"
    );
}

#[test]
fn inline_images_are_inline_and_carry_their_content_id() {
    let parsed = parse("inline-image-cid");

    let parts: Vec<_> = parsed.attachments().collect();
    assert_eq!(
        parts.len(),
        2,
        "two inline PNGs; the third cid: has no part"
    );

    for part in &parts {
        assert_eq!(part.mime_type, "image/png");
        assert_eq!(
            part.disposition,
            Disposition::Inline,
            "a Content-Disposition: inline part is rendered in place"
        );
        assert!(part.is_inline());
        let content_id = part.content_id.as_deref().expect("a Content-ID");
        assert!(
            !content_id.starts_with('<') && !content_id.ends_with('>'),
            "the cid is stored bare so `cid:` URLs match it directly: {content_id:?}"
        );
    }

    let html = parsed.body.html.as_deref().expect("the HTML body");
    for part in &parts {
        let content_id = part.content_id.as_deref().unwrap();
        assert!(
            html.contains(&format!("cid:{content_id}")),
            "the body references this part: {content_id}"
        );
    }
}

#[test]
fn nesting_three_levels_deep_is_flattened_with_mime_part_paths() {
    let parsed = parse("nested-multipart");

    assert!(parsed.body.text.is_some() && parsed.body.html.is_some());

    let marker = parsed
        .attachments()
        .find(|attachment| attachment.filename.as_deref() == Some("marker.png"))
        .expect("the inline image, three levels down");
    assert_eq!(marker.disposition, Disposition::Inline);
    assert_eq!(
        marker.part_id.as_deref(),
        Some("1.2.2"),
        "the MIME part path is what a lazy IMAP fetch asks for"
    );

    let notes = parsed
        .attachments()
        .find(|attachment| attachment.filename.as_deref() == Some("layout-notes.txt"))
        .expect("the trailing attachment");
    assert_eq!(notes.disposition, Disposition::Attachment);
    assert_eq!(notes.part_id.as_deref(), Some("2"));
}

#[test]
fn a_single_part_message_is_mime_part_one() {
    let parsed = mime::parse(
        b"Content-Type: application/pdf\r\n\
          Content-Disposition: attachment; filename=\"a.pdf\"\r\n\r\n\
          %PDF-1.4\r\n",
    );

    let attachment = parsed.attachments().next().expect("the only part");
    assert_eq!(attachment.part_id.as_deref(), Some("1"));
}

#[test]
fn a_calendar_invite_exposes_the_ics_parts() {
    let parsed = parse("calendar-invite");

    assert!(
        parsed
            .parts
            .iter()
            .any(|part| part.attachment.mime_type == "text/calendar"),
        "the METHOD=REQUEST part is reachable, not swallowed as a body"
    );
}

#[test]
fn a_bounce_keeps_its_report_parts() {
    let parsed = parse("bounce-delivery-status");

    let types: Vec<&str> = parsed
        .parts
        .iter()
        .map(|part| part.attachment.mime_type.as_str())
        .collect();
    assert!(
        types.contains(&"message/delivery-status"),
        "the machine-readable part of the bounce: {types:?}"
    );
    assert!(
        types.contains(&"message/rfc822"),
        "and the original message it is about: {types:?}"
    );
}

#[test]
fn a_pgp_signed_message_keeps_the_signature_part_byte_exact() {
    let parsed = parse("pgp-signed");

    let signature = parsed
        .parts
        .iter()
        .find(|part| part.attachment.mime_type == "application/pgp-signature")
        .expect("the detached signature");
    let content = String::from_utf8_lossy(&signature.content);
    assert!(
        content.contains("-----BEGIN PGP SIGNATURE-----")
            && content.contains("-----END PGP SIGNATURE-----"),
        "the armour is preserved verbatim: {content:?}"
    );
}

#[test]
fn a_header_block_that_is_wrong_in_a_dozen_ways_still_yields_headers() {
    let parsed = parse("malformed-headers");

    assert!(parsed.headers.len() > 4, "the parseable fields survive");
    assert!(
        parsed.subject.is_some(),
        "a duplicated Subject resolves to the first one"
    );
    assert!(
        parsed.date.is_none(),
        "an unparseable Date is absent rather than invented"
    );
}

#[test]
fn bare_lf_line_endings_still_produce_a_body() {
    let parsed = parse("malformed-bare-lf");

    assert!(
        !parsed.body.is_empty(),
        "a strict CRLF parser finds nothing here; we must not be one"
    );
}

#[test]
fn a_multipart_truncated_mid_attachment_still_yields_its_first_part() {
    let parsed = parse("malformed-truncated-multipart");

    assert!(
        parsed
            .body
            .text
            .as_deref()
            .is_some_and(|text| !text.trim().is_empty()),
        "delivery was cut short, but the reader still gets what arrived"
    );
}

#[test]
fn a_message_that_ends_after_its_headers_has_no_body() {
    let parsed = parse("headers-only-no-body");

    assert_eq!(
        parsed.subject.as_deref(),
        Some("Nightly build 4412: no changes")
    );
    assert!(parsed.body.is_empty(), "there is nothing after the headers");
    assert!(parsed.preview.is_none());
    assert!(parsed.attachments().next().is_none());
}

#[test]
fn a_message_with_no_message_id_and_no_date_parses_anyway() {
    let parsed = parse("missing-message-id-and-date");

    assert!(parsed.rfc_message_id.is_none(), "nothing is invented");
    assert!(parsed.date.is_none());
    assert!(parsed.to.is_empty());
    assert_eq!(addresses(&parsed.from), ["unattributed@example.com"]);
    assert!(parsed.body.text.is_some());
}

#[test]
fn broken_references_are_salvaged_entry_by_entry() {
    let parsed = parse("broken-references");

    assert_eq!(
        parsed.in_reply_to,
        Some(RfcMessageId::new(
            "<this-message-id-was-never-seen@nowhere.example.invalid>"
        ))
    );

    let references: Vec<String> = parsed
        .references
        .iter()
        .map(|reference| reference.to_string())
        .collect();
    assert!(
        references.contains(&"<harbour-dev.20260302T093345.b2@lists.example.org>".to_owned()),
        "the well-formed entries are kept: {references:?}"
    );
    assert!(
        !references.iter().any(|reference| reference == "<>"),
        "an empty angle-addr is not a reference: {references:?}"
    );
    assert!(
        !references
            .iter()
            .any(|reference| reference.contains("not-an-angle-addr-at-all")),
        "and neither is a bare token: {references:?}"
    );
}

#[test]
fn the_reference_chain_is_what_threading_will_walk() {
    let parsed = parse("list-thread-04-reply-deep");

    assert!(
        parsed.references.len() >= 2,
        "the folded two-entry References chain is parsed whole"
    );
    assert!(parsed.in_reply_to.is_some());
}

#[test]
fn a_list_id_header_is_reduced_to_its_bracketed_identifier() {
    let parsed = parse("list-thread-01-root");

    assert_eq!(
        parsed.list_id.as_deref(),
        Some("harbour-dev.lists.example.org"),
        "the phrase before the bracket is display text, not part of the id"
    );
}

#[test]
fn a_message_outside_any_list_has_no_list_id() {
    let parsed = parse("plain-text-simple");
    assert_eq!(parsed.list_id, None);
}

#[test]
fn list_id_from_text_keeps_the_bracketed_part_and_trims_the_rest() {
    assert_eq!(
        mime::list_id_from_text("Harbour dev <harbour-dev.lists.example.org>"),
        Some("harbour-dev.lists.example.org".to_string())
    );
    assert_eq!(
        mime::list_id_from_text("<no-phrase.example.org>"),
        Some("no-phrase.example.org".to_string()),
        "not every sender includes a display name"
    );
    assert_eq!(
        mime::list_id_from_text("bare.example.org"),
        Some("bare.example.org".to_string()),
        "and some senders send no brackets at all"
    );
    assert_eq!(
        mime::list_id_from_text("   "),
        None,
        "whitespace is nothing"
    );
    assert_eq!(mime::list_id_from_text(""), None);
}

#[test]
fn list_id_from_text_reads_the_first_bracket_outside_a_quoted_phrase() {
    assert_eq!(
        mime::list_id_from_text("\"Dev <team>\" <dev.lists.example.org>"),
        Some("dev.lists.example.org".to_string()),
        "a `<` inside the quoted phrase is not the list-id"
    );
    assert_eq!(
        mime::list_id_from_text("<a.example.org>\r\nList-Id: <b.example.org>\r\n"),
        Some("a.example.org".to_string()),
        "two fields must not run together into one identifier"
    );
}

#[test]
fn disposition_notification_to_is_detected_as_a_read_receipt_request() {
    let raw = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Please confirm\r\n\
Disposition-Notification-To: ada@example.com\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Let me know you got this\r\n";
    let parsed = mime::parse(raw);
    assert!(parsed.read_receipt_requested);
}

#[test]
fn the_older_return_receipt_to_is_also_detected() {
    let raw = b"From: Ada Lovelace <ada@example.com>\r\n\
To: Grace Hopper <grace@example.net>\r\n\
Subject: Please confirm\r\n\
Return-Receipt-To: ada@example.com\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Let me know you got this\r\n";
    let parsed = mime::parse(raw);
    assert!(parsed.read_receipt_requested);
}

#[test]
fn a_message_that_never_asked_for_a_receipt_says_so() {
    let parsed = parse("plain-text-simple");
    assert!(!parsed.read_receipt_requested);
}

fn received_at() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 12, 0, 0).unwrap()
}

#[test]
fn a_parsed_message_becomes_a_domain_message() {
    let parsed = parse("attachment-pdf");
    let expected_subject = parsed.subject.clone();

    let message = parsed.into_message(AccountId::new(7), MailboxId::new(3), received_at());

    assert_eq!(message.account_id, AccountId::new(7));
    assert_eq!(message.mailbox_id, MailboxId::new(3));
    assert_eq!(message.received_at, received_at());
    assert_eq!(message.subject, expected_subject);
    assert!(
        !message.is_persisted(),
        "ids are the storage layer's to hand out"
    );
    assert!(message.has_attachments());
    assert_eq!(message.attachments.len(), 2);
    assert_eq!(message.sync.body_state, BodyState::Full);
    assert!(message.size > 0);
    assert!(message.thread_id.is_none(), "threading has not run yet");
    assert!(
        message.raw_blob_id.is_none(),
        "the blob store has not run yet"
    );
}

#[test]
fn the_date_header_is_kept_separate_from_the_receive_time() {
    let message = parse("plain-text-simple").into_message(
        AccountId::new(1),
        MailboxId::new(1),
        received_at(),
    );

    assert_eq!(
        message.date,
        Some(Utc.with_ymd_and_hms(2026, 2, 10, 9, 15, 0).unwrap()),
        "what the sender claimed"
    );
    assert_eq!(message.received_at, received_at(), "what the server saw");
    assert_eq!(message.best_date(), message.date.unwrap());
}

#[test]
fn a_message_with_no_date_sorts_by_when_it_arrived() {
    let message = parse("missing-message-id-and-date").into_message(
        AccountId::new(1),
        MailboxId::new(1),
        received_at(),
    );

    assert!(message.date.is_none());
    assert_eq!(message.best_date(), received_at());
}

#[test]
fn a_header_only_parse_maps_to_a_header_only_message() {
    let parsed = mime::parse_headers(test_corpus::load("attachment-pdf").bytes());
    let message = parsed.into_message(AccountId::new(1), MailboxId::new(1), received_at());

    assert_eq!(message.sync.body_state, BodyState::HeadersOnly);
    assert!(message.body.is_empty());
    assert!(
        message.subject.is_some() && !message.from.is_empty(),
        "the envelope is still there"
    );
}

#[test]
fn every_attachment_fixture_yields_at_least_one_part() {
    for fixture in test_corpus::by_category(Category::Attachment) {
        let parsed = mime::parse(fixture.bytes());
        assert!(
            parsed.attachments().next().is_some(),
            "{}: tagged `attachment` but no part came out",
            fixture.name()
        );
    }
}

#[test]
fn every_plain_text_fixture_yields_text_unless_it_has_no_body() {
    for fixture in test_corpus::by_category(Category::PlainText) {
        if fixture.name() == "headers-only-no-body" {
            continue; // The one fixture that exists because it has no body.
        }
        let parsed = mime::parse(fixture.bytes());
        assert!(
            parsed.body.text.is_some(),
            "{}: tagged `plain-text` but no text body came out",
            fixture.name()
        );
    }
}

#[test]
fn every_html_fixture_yields_html() {
    for fixture in test_corpus::by_category(Category::Html) {
        let parsed = mime::parse(fixture.bytes());
        assert!(
            parsed.body.html.is_some(),
            "{}: tagged `html` but no HTML body came out",
            fixture.name()
        );
    }
}

#[test]
fn no_fixture_decodes_to_replacement_characters_in_its_subject() {
    for fixture in test_corpus::all() {
        if fixture.name() == "encoded-word-broken" {
            continue; // Its whole point is encoded words that cannot decode.
        }
        let parsed = mime::parse(fixture.bytes());
        if let Some(subject) = parsed.subject.as_deref() {
            assert!(
                !subject.contains('\u{fffd}'),
                "{}: subject decoded to mojibake: {subject:?}",
                fixture.name()
            );
        }
    }
}

const UNPARSEABLE_MULTIPART: &[u8] = &[
    82, 101, 116, 117, 114, 110, 45, 80, 97, 111, 110, 58, 32, 49, 46, 48, 13, 10, 67, 111, 110,
    116, 101, 110, 116, 45, 84, 121, 112, 101, 58, 32, 109, 117, 108, 116, 105, 112, 97, 114, 116,
    47, 0, 0, 0, 0, 0, 0, 0, 114, 101, 112, 111, 59, 13, 116, 119, 10, 9, 98, 111, 117, 110, 100,
    97, 114, 121, 61, 34, 61, 95, 114, 101, 112, 111, 114, 116, 95, 101, 51, 97, 49, 34, 13, 10,
    10, 13, 10, 45, 45, 61, 95, 114, 101, 112, 111, 114, 116, 95, 101, 51, 97, 49, 117, 115, 58,
    32, 97, 49, 13, 10, 67, 111, 110, 116, 101, 110, 116, 45, 84, 121, 112, 101, 58, 32, 109, 101,
    115, 115, 97, 103, 101, 47, 114, 102, 99, 56, 50, 50, 13, 10, 67, 108, 32, 97, 115, 32, 119,
    101, 108, 108, 46, 13, 10, 13, 10, 45, 45, 61, 116, 97, 116, 97, 49, 13, 10, 67, 111, 110, 116,
    101, 110, 116, 45, 84, 121, 112, 101, 58, 32, 109, 101, 115, 115, 97, 103, 101, 47, 114, 102,
    99, 56, 50, 50, 13, 10, 67, 111, 13, 10, 13, 10, 82, 101, 115, 32, 119, 101, 108, 108, 46, 13,
    10, 13, 10, 45, 45, 61, 95, 114, 101, 112, 111, 114, 116, 95, 101, 51, 97, 49, 45, 45, 13, 10,
];

#[test]
fn a_malformed_multipart_does_not_panic_out_of_parse() {
    let parsed = mime::parse(UNPARSEABLE_MULTIPART);
    assert_eq!(
        parsed.size,
        UNPARSEABLE_MULTIPART.len() as u64,
        "the size is what arrived, whether or not it parsed"
    );

    let headers_only = mime::parse_headers(UNPARSEABLE_MULTIPART);
    assert_eq!(headers_only.size, UNPARSEABLE_MULTIPART.len() as u64);
}

#[test]
fn the_reproducer_parses_and_recovers_its_parts() {
    let parsed = mime::try_parse(UNPARSEABLE_MULTIPART).expect("1.0 no longer unwinds here");
    assert!(
        !parsed.parts.is_empty(),
        "the nested messages were dropped: {parsed:?}"
    );

    assert!(
        mime::try_parse_headers(UNPARSEABLE_MULTIPART).is_ok(),
        "headers-only parsing does not reach the multipart walk"
    );
}

#[test]
fn the_reproducer_is_reported_as_damaged() {
    assert!(
        mime::parse(UNPARSEABLE_MULTIPART).encoding_problems,
        "a guessed structure read as a clean message"
    );
}

#[test]
fn a_part_cut_at_a_delimiter_found_mid_line_is_reported() {
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"SEP\"\r\n\r\n\
--SEP\r\nContent-Type: text/plain\r\n\r\none\r\nnot --SEP here\r\ntwo\r\n";
    let parsed = mime::parse(raw);
    assert!(
        parsed.encoding_problems,
        "a body cut short read as whole: {:?}",
        parsed.body.text
    );
}

#[test]
fn a_multipart_without_its_close_delimiter_is_not_reported() {
    let raw = b"From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"SEP\"\r\n\r\n\
--SEP\r\nContent-Type: text/plain\r\n\r\none\r\ntwo\r\n";
    let parsed = mime::parse(raw);
    assert_eq!(parsed.body.text.as_deref(), Some("one\r\ntwo\r\n"));
    assert!(!parsed.encoding_problems);
}

#[test]
fn every_corpus_fixture_still_parses() {
    for fixture in test_corpus::all() {
        assert!(
            mime::try_parse(fixture.bytes()).is_ok(),
            "{} should parse",
            fixture.name()
        );
    }
}

fn nested_multipart(depth: usize) -> Vec<u8> {
    let mut out = String::from("Subject: Nested\r\nMIME-Version: 1.0\r\n");
    for level in 0..depth {
        out.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=\"b{level}\"\r\n\r\n--b{level}\r\n"
        ));
    }
    out.push_str("Content-Type: text/plain\r\n\r\nleaf\r\n");
    for level in (0..depth).rev() {
        out.push_str(&format!("\r\n--b{level}--\r\n"));
    }
    out.into_bytes()
}

#[test]
fn a_deeply_nested_multipart_does_not_overflow_the_stack() {
    let raw = nested_multipart(20_000);
    let parsed = mime::parse(&raw);
    assert_eq!(parsed.size, raw.len() as u64);
}

#[test]
fn the_iterative_walk_numbers_parts_the_way_imap_does() {
    let parsed = parse("attachment-pdf");
    let paths: Vec<String> = parsed
        .parts
        .iter()
        .filter_map(|part| part.attachment.part_id.clone())
        .collect();
    assert!(
        !paths.is_empty(),
        "the fixture should have at least one numbered part"
    );
    for path in &paths {
        assert!(
            path.split('.')
                .all(|segment| segment.parse::<u32>().is_ok()),
            "a part path is dot-separated positive integers: {path}"
        );
    }
}

#[test]
fn a_fetched_section_decodes_from_the_headers_that_explain_it() {
    let entity = b"Content-Type: application/pdf\r\n\
                   Content-Transfer-Encoding: base64\r\n\
                   \r\n\
                   JVBERi0xLjQKJeLjz9MK\r\n";

    let decoded = postio_model::mime::decode_entity(entity).expect("a decodable part");

    assert_eq!(decoded, b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
}

#[test]
fn a_section_with_no_transfer_encoding_is_its_own_bytes() {
    let entity = b"Content-Type: text/csv\r\n\r\nname,size\r\nada,3\r\n";

    let decoded = postio_model::mime::decode_entity(entity).expect("a decodable part");

    assert_eq!(decoded, b"name,size\r\nada,3\r\n");
}

#[test]
fn bytes_that_are_not_an_entity_at_all_decode_to_nothing() {
    assert_eq!(postio_model::mime::decode_entity(b""), None);
}

fn mixed_with(part: &str) -> Vec<u8> {
    format!(
        "From: ada@example.com\r\nMIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"m\"\r\n\r\n\
--m\r\nContent-Type: text/plain\r\n\r\nbody\r\n--m\r\n{part}\r\n--m--\r\n"
    )
    .into_bytes()
}

#[test]
fn a_text_attachment_keeps_the_senders_bytes_whatever_its_charset() {
    let parsed = mime::parse(&mixed_with(
        "Content-Type: text/plain; charset=iso-8859-1; name=\"n.txt\"\r\n\
Content-Disposition: attachment; filename=\"n.txt\"\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\r\ncaf=E9",
    ));
    assert_eq!(parsed.parts.len(), 1);
    assert_eq!(parsed.parts[0].content, b"caf\xe9");
}

#[test]
fn decode_entity_agrees_with_the_full_parse_on_a_text_payload() {
    let decoded = mime::decode_entity(
        b"Content-Type: text/plain; charset=iso-8859-1\r\n\
Content-Transfer-Encoding: quoted-printable\r\n\r\ncaf=E9",
    );
    assert_eq!(decoded.as_deref(), Some(&b"caf\xe9"[..]));
}

#[test]
fn decode_entity_refuses_a_multipart() {
    assert_eq!(
        mime::decode_entity(&mixed_with("Content-Type: text/plain\r\n\r\nx")),
        None
    );
}

#[test]
fn rfc2231_sections_are_joined_before_they_are_converted() {
    let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
        list.iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect()
    };
    assert_eq!(
        mime::parameter_value(
            &pairs(&[("filename*0*", "utf-8''%E2%98"), ("filename*1*", "%95.txt")]),
            "filename"
        )
        .as_deref(),
        Some("\u{2615}.txt")
    );
    assert_eq!(
        mime::parameter_value(&pairs(&[("filename*", "utf-8''O'Brien.txt")]), "filename")
            .as_deref(),
        Some("O'Brien.txt")
    );
}

#[test]
fn an_encoded_character_split_across_two_words_decodes() {
    assert_eq!(
        mime::decode_header_text(b"=?UTF-8?B?4pg=?= =?UTF-8?B?lQ==?="),
        "\u{2615}"
    );
}

#[test]
fn an_unpadded_base64_body_decodes_to_its_end() {
    let parsed = mime::parse(
        b"From: ada@example.com\r\nContent-Type: text/plain\r\n\
Content-Transfer-Encoding: base64\r\n\r\naGVsbG8gd29ybGQ\r\n",
    );
    assert_eq!(parsed.body.text.as_deref(), Some("hello world"));
}

#[test]
fn a_damaged_base64_attachment_is_decoded_not_stored_as_its_own_text() {
    let parsed = mime::parse(&mixed_with(
        "Content-Type: application/octet-stream; name=\"g.bin\"\r\n\
Content-Disposition: attachment\r\nContent-Transfer-Encoding: base64\r\n\r\nAA!!BB**CC",
    ));
    assert_eq!(parsed.parts.len(), 1);
    assert_eq!(parsed.parts[0].content.len(), 4, "AABBCC is four bytes");
    assert!(parsed.encoding_problems, "and the damage is still flagged");
}

#[test]
fn a_named_html_alternative_is_the_html_body() {
    let parsed = mime::parse(
        b"From: ada@example.com\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n\
--a\r\nContent-Type: text/plain\r\n\r\nplain\r\n\
--a\r\nContent-Type: text/html; name=x.html\r\n\r\n<p>h</p>\r\n--a--\r\n",
    );
    assert_eq!(parsed.body.text.as_deref(), Some("plain"));
    assert_eq!(parsed.body.html.as_deref(), Some("<p>h</p>"));
    assert!(parsed.parts.is_empty(), "{:?}", parsed.parts);
}

#[test]
fn an_html_alternative_disposed_as_an_attachment_stays_a_file() {
    let parsed = mime::parse(
        b"From: ada@example.com\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n\
--a\r\nContent-Type: text/plain\r\n\r\nplain\r\n\
--a\r\nContent-Type: text/html\r\nContent-Disposition: attachment; filename=x.html\r\n\r\n\
<p>h</p>\r\n--a--\r\n",
    );
    assert_eq!(parsed.body.html, None);
    assert_eq!(parsed.parts.len(), 1);
    assert_eq!(parsed.parts[0].attachment.mime_type, "text/html");
}

#[test]
fn a_related_part_without_a_disposition_is_inline() {
    let parsed = mime::parse(
        b"From: ada@example.com\r\nContent-Type: multipart/related; boundary=r\r\n\r\n\
--r\r\nContent-Type: text/html\r\n\r\n<p>x</p>\r\n\
--r\r\nContent-Type: image/png\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0K\r\n--r--\r\n",
    );
    assert_eq!(parsed.parts.len(), 1);
    assert_eq!(parsed.parts[0].attachment.disposition, Disposition::Inline);
}

#[test]
fn a_mixed_image_without_a_disposition_is_an_attachment() {
    let parsed = mime::parse(&mixed_with(
        "Content-Type: image/jpeg; name=\"p.jpg\"\r\nContent-Transfer-Encoding: base64\r\n\r\n/9j/4AAQ",
    ));
    assert_eq!(parsed.parts.len(), 1);
    assert_eq!(
        parsed.parts[0].attachment.disposition,
        Disposition::Attachment
    );
}

#[test]
fn header_names_keep_their_spelling_and_a_spaced_name_is_not_from() {
    let parsed = mime::parse(
        b"Fr om: evil@example.com\r\nFrom: ada@example.com\r\nlist-id: <l.example.com>\r\n\r\nx",
    );
    assert_eq!(addresses(&parsed.from), vec!["ada@example.com"]);
    assert!(parsed.headers.iter().any(|header| header.name == "list-id"));
    assert_eq!(parsed.list_id.as_deref(), Some("l.example.com"));
}

#[test]
fn a_message_that_is_only_headers_has_an_empty_body_and_no_caveat() {
    let parsed = mime::parse(b"From: ada@example.com\r\nSubject: nothing\r\n");
    assert_eq!(parsed.body.text.as_deref(), Some(""));
    assert!(!parsed.encoding_problems);
}

#[test]
fn a_one_digit_zone_offset_is_read_as_hours() {
    let parsed =
        mime::parse(b"From: ada@example.com\r\nDate: Mon, 5 Oct 2026 10:00:00 +5\r\n\r\nx");
    assert_eq!(
        parsed.date,
        Utc.with_ymd_and_hms(2026, 10, 5, 5, 0, 0).single()
    );
}
