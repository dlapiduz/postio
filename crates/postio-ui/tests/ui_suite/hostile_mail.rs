//! Hostile mail in the reader (#1770): what the header and the notice band
//! show for a message built to look like something it is not.
//!
//! Both halves read the message the way every frontend does, through the
//! model's parser, and assert on the strings the reader draws, because the
//! defect lived between the layers: the parser reported a subject with a
//! line break in it, and each frontend set it in a wrapping label.

use chrono::{Local, TimeZone, Utc};
use postio_model::mime::{self, ParsedMessage};
use postio_model::promoted::PromotedHeaders;
use postio_model::test_corpus;
use postio_ui::reader::header::MessageHeader;
use postio_ui::unsubscribe;

const HOSTILE: [&str; 2] = [
    "encoded-word-crlf-in-header",
    "encoded-word-control-chars-in-header",
];

fn parse(name: &str) -> ParsedMessage {
    mime::parse(test_corpus::load(name).bytes())
}

fn header(parsed: &ParsedMessage) -> MessageHeader {
    MessageHeader::of(
        &parsed.from,
        &parsed.to,
        &parsed.cc,
        parsed.subject.as_deref(),
        Utc.with_ymd_and_hms(2026, 3, 1, 10, 15, 0).unwrap(),
        Local::now(),
    )
}

fn breaks_the_line(c: char) -> bool {
    c.is_control() || c == '\u{2028}' || c == '\u{2029}'
}

#[test]
fn an_injected_header_is_drawn_as_part_of_the_subject_not_as_a_line_of_its_own() {
    let drawn = header(&parse("encoded-word-crlf-in-header"));
    assert_eq!(
        drawn.subject, "Invoice attached Bcc: mallory@sender.invalid",
        "the forged Bcc has to read as the odd subject it is, on one line"
    );
    assert_eq!(
        drawn.from,
        "Mallory X-Injected-By-Name: yes <mallory@sender.invalid>"
    );

    for name in HOSTILE {
        let drawn = header(&parse(name));
        for (what, line) in [("subject", &drawn.subject), ("from", &drawn.from)] {
            assert!(
                !line.chars().any(breaks_the_line),
                "{name}: the reader's {what} carries a line break or control \
                 character: {line:?}"
            );
        }
    }
}

#[test]
fn hostile_mail_that_is_not_list_mail_is_offered_no_unsubscribe_band() {
    for name in HOSTILE {
        let parsed = parse(name);
        let offered = PromotedHeaders::from_headers(&parsed.headers).unsubscribe_offered;
        assert_eq!(
            unsubscribe::banner(None, parsed.list_id.as_deref(), Some(offered), &parsed.from),
            None,
            "{name} has neither List-Id nor List-Unsubscribe, so offering to \
             unsubscribe from its sender's domain is a band on exactly the \
             mail least worth trusting"
        );
    }
    // The positive control: the same rule does offer the band on real list
    // mail, so the `None` above is the rule speaking, not a broken fixture.
    let list = parse("html-newsletter");
    let offered = PromotedHeaders::from_headers(&list.headers).unsubscribe_offered;
    assert!(
        unsubscribe::banner(None, list.list_id.as_deref(), Some(offered), &list.from).is_some(),
        "a newsletter with list headers should still be offered the band"
    );
}
