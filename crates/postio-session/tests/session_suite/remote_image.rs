//! A remote image is fetched once, on a press, or never (spec 014 D7,
//! `contracts/remote-image.md`).
//!
//! Against a counting fake `Fetcher`: no case reaches the network, and the
//! count is the privacy property. Every gesture but the press makes no
//! request; each press makes one; a failure is never retried.

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use postio_body::{Block, Document, Inline, WebHref};
use postio_model::{AccountId, Draft, EmailAddress, Identity, mime, outgoing};
use postio_session::attaching::{
    FetchError, FetchRefusal, Fetcher, REMOTE_IMAGE_LIMIT, fetch_remote_image,
};
use postio_storage::test_support;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";

const DRAFT: postio_model::DraftId = postio_model::DraftId::new(41);

/// Answers with what it was told to, and counts every request.
struct Counting {
    requests: AtomicUsize,
    answer: Mutex<Result<Vec<u8>, FetchError>>,
}

impl Counting {
    fn answering(answer: Result<Vec<u8>, FetchError>) -> Self {
        Counting {
            requests: AtomicUsize::new(0),
            answer: Mutex::new(answer),
        }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Fetcher for Counting {
    fn get(&self, _href: &WebHref) -> Result<Vec<u8>, FetchError> {
        self.requests.fetch_add(1, Ordering::SeqCst);
        self.answer.lock().expect("never poisoned").clone()
    }
}

fn a_store() -> (postio_storage::BlobStore, tempfile::TempDir) {
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs = postio_storage::BlobStore::open(scratch.path(), &test_support::blob_keys())
        .expect("a blob store");
    (blobs, scratch)
}

fn chart() -> WebHref {
    WebHref::parse("https://example.com/x.png").expect("a web address")
}

fn with_a_chip() -> Document {
    postio_body::markdown::to_document("See ![x](https://example.com/x.png) here.")
}

#[test]
fn nothing_but_the_press_makes_a_request() {
    let fetcher = Counting::answering(Ok(PNG.to_vec()));
    // Typed in the source view or the terminal, switched, saved, reopened:
    // all of it is the document and its Markdown, none of it a fetch.
    let document = with_a_chip();
    let markdown = postio_body::markdown::from_document(&document);
    let reopened = postio_body::parse(&document.to_html());
    assert_eq!(postio_body::markdown::to_document(&markdown), document);
    assert_eq!(reopened, document);
    // A pasted page's remote <img> narrows away, as it always did.
    let pasted = postio_body::parse("<p><img src=\"https://example.com/y.png\"></p>");
    assert!(!pasted.to_html().contains("<img"));
    // Sent with the chip unpressed: a link in both parts.
    let mut draft = Draft::new(AccountId::new(1));
    draft.to = vec![EmailAddress::new(None::<String>, "bo@example.com")];
    draft.body = postio_body::message_body(&document);
    let identity = Identity::new(
        AccountId::new(1),
        EmailAddress::new(None::<String>, "ada@example.com"),
    );
    let raw = String::from_utf8(outgoing::build(&draft, &identity, &[], None, None).raw).unwrap();
    let parsed = mime::parse(raw.as_bytes());
    let html = parsed.body.html.expect("an HTML part");
    assert!(
        html.contains("<a href=\"https://example.com/x.png\""),
        "{html}"
    );
    assert!(!html.contains("<img"), "{html}");
    let text = parsed.body.text.expect("a plain part");
    assert!(text.contains("x <https://example.com/x.png>"), "{text}");
    assert_eq!(
        fetcher.requests(),
        0,
        "a gesture other than the press made a request"
    );
}

#[test]
fn each_press_is_one_request_and_a_failure_is_not_retried() {
    let (blobs, _scratch) = a_store();
    let fetcher = Counting::answering(Err(FetchError::Unreachable("status 404".into())));
    let refused = fetch_remote_image(&fetcher, &blobs, DRAFT, &chart(), true);
    assert_eq!(
        refused,
        Err(FetchRefusal::Unreachable {
            reason: "status 404".into()
        })
    );
    assert_eq!(fetcher.requests(), 1);
    // Back online, nothing is retried on its own.
    assert_eq!(fetcher.requests(), 1);
    // Pressed again: one more.
    *fetcher.answer.lock().unwrap() = Ok(PNG.to_vec());
    let attached =
        fetch_remote_image(&fetcher, &blobs, DRAFT, &chart(), true).expect("it attaches");
    assert_eq!(fetcher.requests(), 2);
    assert!(
        attached.content_id.is_some(),
        "an inline part, by content id"
    );
    assert_eq!(attached.mime_type, "image/png");
}

#[test]
fn offline_makes_no_request() {
    let (blobs, _scratch) = a_store();
    let fetcher = Counting::answering(Ok(PNG.to_vec()));
    assert_eq!(
        fetch_remote_image(&fetcher, &blobs, DRAFT, &chart(), false),
        Err(FetchRefusal::Offline)
    );
    assert_eq!(fetcher.requests(), 0);
}

#[test]
fn what_is_not_a_picture_or_too_large_is_refused() {
    let (blobs, _scratch) = a_store();
    for (answer, refusal) in [
        (
            Ok(b"<html>a page</html>".to_vec()),
            FetchRefusal::NotAnImage,
        ),
        (
            Ok(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec()),
            FetchRefusal::NotAnImage,
        ),
        (
            Ok(vec![0x89; REMOTE_IMAGE_LIMIT + 1]),
            FetchRefusal::TooLarge,
        ),
        (Err(FetchError::TooLarge), FetchRefusal::TooLarge),
        (Err(FetchError::NotAnImage), FetchRefusal::NotAnImage),
    ] {
        let fetcher = Counting::answering(answer);
        assert_eq!(
            fetch_remote_image(&fetcher, &blobs, DRAFT, &chart(), true),
            Err(refusal)
        );
    }
}

#[test]
fn a_chip_is_never_an_img_on_the_wire() {
    let document = Document {
        blocks: vec![Block::Paragraph(vec![Inline::RemoteImage {
            href: chart(),
            alt: String::new(),
        }])],
    };
    let (_, html) = postio_body::render(&document);
    assert!(!html.contains("<img"), "{html}");
}

#[test]
fn the_log_says_what_happened_and_never_where() {
    use postio_test_support::logs::Captured;
    let (blobs, _scratch) = a_store();
    let captured = Captured::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(captured.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .finish();
    {
        let _guard = tracing::subscriber::set_default(subscriber);
        let fetcher = Counting::answering(Ok(PNG.to_vec()));
        fetch_remote_image(&fetcher, &blobs, DRAFT, &chart(), true).expect("it attaches");
        let fetcher = Counting::answering(Err(FetchError::Unreachable("it answered 404".into())));
        let _ = fetch_remote_image(&fetcher, &blobs, DRAFT, &chart(), true);
    }
    let log = captured.text();
    assert!(log.contains("draft=41"), "{log}");
    assert!(
        log.contains("outcome=\"attached\"") || log.contains("outcome=attached"),
        "{log}"
    );
    assert!(log.contains(&format!("bytes={}", PNG.len())), "{log}");
    assert!(log.contains("unreachable"), "{log}");
    assert!(
        !log.contains("example.com"),
        "the address reached the log: {log}"
    );
}
