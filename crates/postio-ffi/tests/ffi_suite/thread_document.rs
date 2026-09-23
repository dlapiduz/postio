//! A conversation as one document, as the Mac draws it (#1595, ADR 0032).
//!
//! The composition is `postio_ui::reader::thread::compose`'s and is tested
//! there. What is tested here is the boundary's half: that it gathers the
//! thread from the store the way GTK's pane does -- every message in date
//! order, each body loaded or said to be coming, the per-sender decision made
//! against the allow list the Privacy pane edits, the user's own messages
//! marked -- and says where each message is in the page.

use chrono::{DateTime, TimeZone, Utc};
use postio_ffi::{Session, SessionOptions};
use postio_model::{AccountId, BodyState, EmailAddress, MailboxId, Message, Thread};
use postio_storage::repository::{MessageRepository, StoredBody, ThreadRepository};
use postio_storage::test_support;

fn at(seconds: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

/// One message of the thread from `address`, with `html` as its body when
/// there is one.
async fn message(
    connection: &postio_storage::Checkout,
    account: AccountId,
    mailbox: MailboxId,
    address: &str,
    seconds: i64,
    html: Option<&str>,
) -> Message {
    let mut message = Message::new(account, mailbox, at(seconds));
    message.subject = Some("The gate".to_owned());
    message.from = vec![EmailAddress::new(None::<&str>, address)];
    message.to = vec![EmailAddress::new(None::<&str>, "test@example.com")];
    message.preview = Some(format!("Snippet {seconds}"));
    let repository = MessageRepository::new(connection);
    repository.create(&mut message).await.expect("a message");
    if let Some(html) = html {
        repository
            .set_body(
                message.id,
                &StoredBody {
                    text: None,
                    html: Some(html.to_owned()),
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("a body");
    }
    message
}

/// Ada writes, the user answers, Quinn writes last and has no body yet.
async fn a_thread() -> (std::sync::Arc<Session>, i64, Vec<i64>) {
    let database = test_support::memory().await;
    let (thread, ids) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut thread = Thread::new(account.id);
        thread.subject = Some("the gate".to_owned());
        let threads = ThreadRepository::new(&connection);
        threads.create(&mut thread).await.expect("a thread");

        // Out of order on purpose: date order is the boundary's to apply.
        let quinn = message(
            &connection,
            account.id,
            inbox,
            "quinn@example.org",
            300,
            None,
        )
        .await;
        let ada = message(
            &connection,
            account.id,
            inbox,
            "ada@example.com",
            100,
            Some(r#"<p>Friday?</p><img src="https://pixels.example.com/ada.png" alt="">"#),
        )
        .await;
        let mine = message(
            &connection,
            account.id,
            inbox,
            "test@example.com",
            200,
            Some(r#"<p>Friday works.</p><img src="https://pixels.example.com/me.png" alt="">"#),
        )
        .await;
        for message in [&ada, &mine, &quinn] {
            threads
                .add_message(thread.id, message.id)
                .await
                .expect("add");
        }
        (
            thread.id.get(),
            vec![ada.id.get(), mine.id.get(), quinn.id.get()],
        )
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");
    (session, thread, ids)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_is_one_document_with_every_message_in_date_order() {
    let (session, thread, ids) = a_thread().await;

    let document = session.thread_document(thread, Vec::new()).await;

    assert_eq!(
        document
            .messages
            .iter()
            .map(|entry| entry.message)
            .collect::<Vec<_>>(),
        ids,
        "not the thread in the order it was written"
    );
    for entry in &document.messages {
        assert!(
            document.html.contains(&format!("id=\"{}\"", entry.anchor)),
            "message {} has an anchor the page does not carry",
            entry.message
        );
    }
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn each_message_offers_its_own_verbs() {
    // FR-009: without a verb per message there is no way to reply to an
    // older one. The link names the message, and the pane routes on it.
    let (session, thread, ids) = a_thread().await;
    let document = session.thread_document(thread, Vec::new()).await;
    assert!(document.html.contains(&format!("postio-reply:{}", ids[0])));
    assert!(
        document
            .html
            .contains(&format!("postio-forward:{}", ids[0]))
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_sender_the_user_allowed_keeps_their_images_in_the_thread() {
    // The allow list the Privacy pane edits, read on this side: the Mac does
    // not decide per sender, it is told.
    let (session, thread, _) = a_thread().await;
    session.allow_sender("ada@example.com".to_owned());

    let document = session.thread_document(thread, Vec::new()).await;

    assert!(document.html.contains("pixels.example.com/ada.png"));
    assert!(
        !document.html.contains("pixels.example.com/me.png"),
        "an image from a sender nobody allowed rode in on Ada's grant"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_message_says_its_body_is_still_coming() {
    // It opens even with no body, so the plate has somewhere to appear: a
    // thread whose last message would not open said nothing about why.
    let (session, thread, _) = a_thread().await;
    let document = session.thread_document(thread, Vec::new()).await;
    assert_eq!(
        document.html.matches("role=\"status\"").count(),
        1,
        "{}",
        document.html
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_users_own_message_is_marked_as_theirs() {
    // Canvas 18's outline (#1241), from the account's own address.
    let (session, thread, ids) = a_thread().await;
    let document = session.thread_document(thread, Vec::new()).await;
    let anchor = &document.messages[1].anchor;
    assert_eq!(document.messages[1].message, ids[1]);
    assert!(
        document.html.contains(&format!(
            "class=\"postio-message postio-mine\" id=\"{anchor}\""
        )),
        "the user's own reply is drawn as somebody else's"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_nobody_can_read_is_an_empty_document() {
    let (session, _, _) = a_thread().await;
    let document = session.thread_document(987_654, Vec::new()).await;
    assert!(document.messages.is_empty());
    session.shutdown();
}
