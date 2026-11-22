//! What a formatted draft is when it leaves (spec 014 T021, FR-018).
//!
//! The document is rendered to a `MessageBody` by `postio_body`, the one
//! function every composer sends through, and built into the bytes handed
//! to SMTP `DATA` by `postio_model::outgoing`. Asserted on those bytes,
//! re-parsed, because that is what a recipient's client is given.

use postio_model::{AccountId, Draft, EmailAddress, Identity, mime, outgoing};

fn sent(document: &postio_body::Document) -> String {
    let mut draft = Draft::new(AccountId::new(1));
    draft.subject = "Plan".to_owned();
    draft.to = vec![EmailAddress::new(None::<String>, "bo@example.com")];
    draft.body = postio_body::message_body(document);
    let identity = Identity::new(
        AccountId::new(1),
        EmailAddress::new(Some("Ada"), "ada@example.com"),
    );
    let built = outgoing::build(&draft, &identity, &[], None, None);
    String::from_utf8(built.raw).expect("the message is UTF-8")
}

#[test]
fn a_formatted_document_goes_out_as_both_parts() {
    let golden = postio_body::golden::golden();
    let raw = sent(&golden);
    assert!(raw.contains("multipart/alternative"), "{raw}");
    assert!(raw.contains("format=\"flowed\""), "{raw}");
    assert!(raw.contains("text/html"), "{raw}");

    let parsed = mime::parse(raw.as_bytes());
    let (text, html) = postio_body::render(&golden);
    // The wire's line ends are CRLF, inside a `<pre>` as anywhere.
    assert_eq!(
        parsed
            .body
            .html
            .as_deref()
            .map(|html| html.trim_end().replace("\r\n", "\n")),
        Some(html),
        "the HTML part is not the golden HTML"
    );
    let received = parsed.body.text.expect("a plain part");
    assert!(
        received.contains("~~struck~~") && received.contains("- [x] done task"),
        "the plain part lost its markers: {received:?}"
    );
    assert!(text.contains("| Widget |   3 |"));
}

#[test]
fn a_plain_document_goes_out_as_flowed_text_alone() {
    let plain = postio_body::Document::from_text("Hello Bo,\n\nSix is fine.");
    let raw = sent(&plain);
    assert!(!raw.contains("multipart/"), "{raw}");
    assert!(!raw.contains("text/html"), "{raw}");
    assert!(raw.contains("text/plain"), "{raw}");
    assert!(raw.contains("format=\"flowed\""), "{raw}");
}
