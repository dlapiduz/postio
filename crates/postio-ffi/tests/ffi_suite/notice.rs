//! The blocked-images notice, and the two standing grants behind it (#1274).
//!
//! Remote images are blocked by default and the reader says so. What it says
//! — how many, and about whom — has to cross, and so does the decision to
//! stop asking: a per-sender or per-domain grant that lived in one frontend
//! would be a second answer to "may this sender see me", which is the one
//! place two implementations are least acceptable.

use chrono::Utc;
use postio_ffi::{RemoteImagesFfi, Session, SessionOptions};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// A session over a store holding one message from `sender` whose body
/// points at `images` remote pictures.
fn a_message_with_images(sender: &str, images: usize) -> (std::sync::Arc<Session>, i64) {
    let (session, messages) = a_store_with(&[(sender, images)]);
    (session, messages[0])
}

/// A session over a store holding one message that reads as bulk mail —
/// nested tables, which is the arrangement reader view exists to reduce.
fn a_bulk_message(sender: &str) -> (std::sync::Arc<Session>, i64) {
    let (session, messages) = a_store_with_html(&[(
        sender,
        "<table><tr><td><table><tr><td><p>Sale</p>\
         <img src=\"https://tracker.example/1.png\"></td></tr></table></td></tr></table>",
    )]);
    (session, messages[0])
}

/// A session over a store holding one message per `(sender, images)` pair.
fn a_store_with(senders: &[(&str, usize)]) -> (std::sync::Arc<Session>, Vec<i64>) {
    let bodies: Vec<(&str, String)> = senders
        .iter()
        .map(|(sender, images)| {
            (
                *sender,
                (0..*images)
                    .map(|n| format!("<p>hello <img src=\"https://tracker.example/{n}.png\"></p>"))
                    .collect::<String>(),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str)> = bodies
        .iter()
        .map(|(sender, html)| (*sender, html.as_str()))
        .collect();
    a_store_with_html(&borrowed)
}

/// As [`a_store_with`], with the body written out.
fn a_store_with_html(senders: &[(&str, &str)]) -> (std::sync::Arc<Session>, Vec<i64>) {
    let database = test_support::memory();
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let ids = {
        let connection = database.connection().expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection);
        let repository = MessageRepository::new(&connection);
        let mut ids = Vec::new();
        for (sender, html) in senders {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.from = vec![EmailAddress::new(Some("Notices"), *sender)];
            let id = repository.create(&mut message).expect("a message");
            let html = (*html).to_owned();
            repository
                .set_body(
                    id,
                    &StoredBody {
                        text: None,
                        html: Some(html),
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::message::BodyState::Full,
                )
                .expect("the body is stored");
            ids.push(id.get());
        }
        ids
    };

    let session =
        Session::open(SessionOptions::in_memory_with(database).with_blobs_for_test(blobs, scratch))
            .expect("a session over the store");
    (session, ids)
}

#[test]
fn a_message_pointing_at_pictures_says_how_many_were_blocked() {
    let (session, message) = a_message_with_images("notices@relay.example.net", 6);
    let notice = session
        .reader_notice(message)
        .expect("a notice, because something was held back");

    assert!(
        notice.summary.starts_with("6 remote images"),
        "the notice names the count: {}",
        notice.summary
    );
    assert_eq!(notice.sender, "notices@relay.example.net");
    assert_eq!(notice.domain, "relay.example.net");
    assert!(!notice.allowed);
}

#[test]
fn a_message_with_nothing_held_back_has_no_notice_at_all() {
    // A notice with nothing to report should not be on screen: it teaches
    // people to dismiss the one that matters.
    let (session, message) = a_message_with_images("ada@example.com", 0);
    assert!(session.reader_notice(message).is_none());
}

#[test]
fn allowing_a_sender_lasts_and_the_notice_says_so() {
    let (session, message) = a_message_with_images("ada@example.com", 3);
    assert!(!session.reader_notice(message).expect("a notice").allowed);

    session.allow_sender("ada@example.com".to_owned());

    let notice = session.reader_notice(message).expect("a notice");
    assert!(
        notice.allowed,
        "the reader stops asking about a sender who has been allowed"
    );
}

#[test]
fn allowing_a_domain_covers_the_addresses_under_it() {
    // The case one address at a time cannot solve: a service that never
    // sends twice from the same address.
    let (session, message) = a_message_with_images("notices-3a7f@relay.example.net", 2);
    session.allow_domain("relay.example.net".to_owned());

    assert!(session.reader_notice(message).expect("a notice").allowed);
}

#[test]
fn a_grant_is_about_this_sender_and_nobody_else() {
    // One store, two senders at the same domain: allowing an address must
    // not quietly allow everybody who shares its domain, which is the
    // difference between the two menu items.
    let (session, messages) = a_store_with(&[("ada@example.com", 2), ("bo@example.com", 2)]);
    session.allow_sender("ada@example.com".to_owned());

    assert!(
        session
            .reader_notice(messages[0])
            .expect("a notice")
            .allowed
    );
    assert!(
        !session
            .reader_notice(messages[1])
            .expect("a notice")
            .allowed,
        "allowing one address must not allow the rest of its domain"
    );
}

#[test]
fn the_document_still_blocks_until_the_frontend_asks_for_allowed() {
    // The grant is a *decision*, not a rendering: what the reader renders is
    // still the frontend's call, so a build that ignored the notice cannot
    // accidentally leak by consulting the list twice.
    let (session, message) = a_message_with_images("ada@example.com", 2);
    session.allow_sender("ada@example.com".to_owned());

    let blocked = session.reader_document(message, RemoteImagesFfi::Blocked, false);
    assert!(
        !blocked.contains("tracker.example"),
        "asked for blocked, blocked it is"
    );
}

#[test]
fn view_original_leaves_reader_view_for_this_message_and_no_further() {
    // The one gesture that may leave reader view (canvas 26). Per message and
    // per view: nothing is remembered, so the next message opens reduced.
    // Bulk mail, which is what reader view is *for*: nested tables are what
    // a campaign template does and a person writing mail does not.
    let (session, message) = a_bulk_message("notices@relay.example.net");

    let reduced = session.reader_document(message, RemoteImagesFfi::Blocked, false);
    let original = session.reader_document(message, RemoteImagesFfi::Blocked, true);

    assert_ne!(
        reduced, original,
        "asking for the original has to actually change what is drawn"
    );
    assert_eq!(
        session.reader_document(message, RemoteImagesFfi::Blocked, false),
        reduced,
        "and asking again for the ordinary rendering gets it back"
    );
}
