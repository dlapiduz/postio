use chrono::Utc;
use postio_ffi::{RemoteImagesFfi, Session, SessionOptions};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

async fn a_message_with_images(sender: &str, images: usize) -> (std::sync::Arc<Session>, i64) {
    let (session, messages) = a_store_with(&[(sender, images)]).await;
    (session, messages[0])
}

async fn a_bulk_message(sender: &str) -> (std::sync::Arc<Session>, i64) {
    let (session, messages) = a_store_with_html(&[(
        sender,
        "<table><tr><td><table><tr><td><p>Sale</p>\
         <img src=\"https://tracker.example/1.png\"></td></tr></table></td></tr></table>",
    )])
    .await;
    (session, messages[0])
}

async fn a_store_with(senders: &[(&str, usize)]) -> (std::sync::Arc<Session>, Vec<i64>) {
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
    a_store_with_html(&borrowed).await
}

async fn a_store_with_html(senders: &[(&str, &str)]) -> (std::sync::Arc<Session>, Vec<i64>) {
    let database = test_support::memory().await;
    let scratch = tempfile::tempdir().expect("a scratch directory");
    let blobs =
        postio_storage::BlobStore::open(scratch.path(), &postio_storage::test_support::blob_keys())
            .expect("a blob store");

    let ids = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let repository = MessageRepository::new(&connection);
        let mut ids = Vec::new();
        for (sender, html) in senders {
            let mut message = Message::new(account.id, inbox, Utc::now());
            message.from = vec![EmailAddress::new(Some("Notices"), *sender)];
            let id = repository.create(&mut message).await.expect("a message");
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
                .await
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

#[tokio::test(flavor = "multi_thread")]
async fn a_message_pointing_at_pictures_says_how_many_were_blocked() {
    let (session, message) = a_message_with_images("notices@relay.example.net", 6).await;
    let notice = session
        .reader_notice(message)
        .await
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

#[tokio::test(flavor = "multi_thread")]
async fn a_message_with_nothing_held_back_has_no_notice_at_all() {
    let (session, message) = a_message_with_images("ada@example.com", 0).await;
    assert!(session.reader_notice(message).await.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn allowing_a_sender_lasts_and_the_notice_says_so() {
    let (session, message) = a_message_with_images("ada@example.com", 3).await;
    assert!(
        !session
            .reader_notice(message)
            .await
            .expect("a notice")
            .allowed
    );

    session.allow_sender("ada@example.com".to_owned());

    let notice = session.reader_notice(message).await.expect("a notice");
    assert!(
        notice.allowed,
        "the reader stops asking about a sender who has been allowed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn allowing_a_domain_covers_the_addresses_under_it() {
    let (session, message) = a_message_with_images("notices-3a7f@relay.example.net", 2).await;
    session.allow_domain("relay.example.net".to_owned());

    assert!(
        session
            .reader_notice(message)
            .await
            .expect("a notice")
            .allowed
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_grant_is_about_this_sender_and_nobody_else() {
    let (session, messages) = a_store_with(&[("ada@example.com", 2), ("bo@example.com", 2)]).await;
    session.allow_sender("ada@example.com".to_owned());

    assert!(
        session
            .reader_notice(messages[0])
            .await
            .expect("a notice")
            .allowed
    );
    assert!(
        !session
            .reader_notice(messages[1])
            .await
            .expect("a notice")
            .allowed,
        "allowing one address must not allow the rest of its domain"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_document_still_blocks_until_the_frontend_asks_for_allowed() {
    // The grant is a decision, not a rendering: what the reader renders is still the frontend's
    // call.
    let (session, message) = a_message_with_images("ada@example.com", 2).await;
    session.allow_sender("ada@example.com".to_owned());

    let blocked = session.reader_document(message, RemoteImagesFfi::Blocked, false);
    assert!(
        !blocked.await.contains("tracker.example"),
        "asked for blocked, blocked it is"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_bulk_message_opens_as_its_sender_built_it() {
    // Reader view is a command away, never the default; the Mac opened bulk mail reduced, so one
    // application had two rules.
    let (session, message) = a_bulk_message("notices@relay.example.net").await;

    let opened = session
        .reader_document(message, RemoteImagesFfi::Blocked, false)
        .await;

    assert!(
        opened.contains(postio_ui::reader::document::SENDERS_SHEET_CLASS),
        "the original of bulk mail is drawn on its sender's sheet"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn reader_view_is_asked_for_one_message_at_a_time() {
    let (session, message) = a_bulk_message("notices@relay.example.net").await;

    let reduced = session
        .reader_document(message, RemoteImagesFfi::Blocked, true)
        .await;
    let opened = session
        .reader_document(message, RemoteImagesFfi::Blocked, false)
        .await;

    assert_ne!(
        reduced, opened,
        "asking for reader view has to change what is drawn"
    );
    assert!(
        !reduced.contains(postio_ui::reader::document::SENDERS_SHEET_CLASS),
        "a reduced message is on Postio's own page, not the sender's"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_notice_carries_the_counts_the_parts_panel_needs() {
    // The parts panel needs counts, not the sentence: parsing "6 remote images blocked" apart would
    // be a second reader of a string for people.
    let (session, message) = a_message_with_images("notices@relay.example.net", 6).await;
    let notice = session
        .reader_notice(message)
        .await
        .expect("the reader held something back");

    assert!(
        notice.remote_images > 0,
        "the notice says something was blocked and reports none of it: {}",
        notice.summary
    );
    assert!(
        notice.summary.contains(&notice.remote_images.to_string()),
        "the count and the sentence disagree: {} vs {}",
        notice.remote_images,
        notice.summary
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn always_show_images_grants_the_sender_the_notice_names() {
    let (session, messages) = a_store_with(&[("ada@example.com", 2), ("bo@example.com", 2)]).await;

    assert!(
        session.always_show_images_for(messages[0]).await,
        "a notice was up, so the command acted"
    );

    assert!(
        session
            .reader_notice(messages[0])
            .await
            .expect("a notice")
            .allowed
    );
    assert!(
        !session
            .reader_notice(messages[1])
            .await
            .expect("a notice")
            .allowed,
        "the grant is the sender's, not the domain's"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn always_show_images_does_nothing_where_the_notice_offers_nothing() {
    // A key on a message with nothing held back does nothing; a grant there would be consent nobody
    // was asked for.
    let (session, message) = a_message_with_images("ada@example.com", 0).await;

    assert!(!session.always_show_images_for(message).await);
    assert!(
        session.remote_image_grants().is_empty(),
        "no grant was written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn always_show_images_for_a_sender_already_allowed_is_not_a_second_grant() {
    let (session, message) = a_message_with_images("ada@example.com", 3).await;
    session.allow_sender("ada@example.com".to_owned());

    assert!(
        !session.always_show_images_for(message).await,
        "the notice no longer asks, so there is nothing for the key to press"
    );
}
