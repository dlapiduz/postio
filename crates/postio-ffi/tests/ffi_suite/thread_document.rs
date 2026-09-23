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

#[test]
fn a_verb_link_names_its_verb_and_its_message() {
    // What the pane asks of every navigation before anything reaches the
    // browser: the parse is `postio_ui::reader::thread::verb_of`'s, the same
    // rule GTK's reader applies.
    use postio_ffi::{ThreadVerbFfi, ThreadVerbKindFfi, thread_verb};
    assert_eq!(
        thread_verb("postio-reply:42".to_owned()),
        Some(ThreadVerbFfi {
            kind: ThreadVerbKindFfi::Reply,
            message: 42
        })
    );
    assert_eq!(
        thread_verb("postio-allow:7".to_owned()),
        Some(ThreadVerbFfi {
            kind: ThreadVerbKindFfi::Allow,
            message: 7
        })
    );
    assert_eq!(thread_verb("https://example.com/".to_owned()), None);
    assert_eq!(thread_verb("postio-reply:not-a-message".to_owned()), None);
}

#[tokio::test(flavor = "multi_thread")]
async fn each_anchor_carries_the_sender_the_show_link_would_allow() {
    // `Show` names a message; what it grants is that message's sender.
    let (session, thread, _) = a_thread().await;
    let document = session.thread_document(thread, Vec::new()).await;
    assert_eq!(
        document
            .messages
            .iter()
            .map(|entry| entry.address.as_str())
            .collect::<Vec<_>>(),
        vec!["ada@example.com", "test@example.com", "quinn@example.org"]
    );
    session.shutdown();
}

#[test]
fn the_document_scripts_are_the_shared_ones() {
    // The Mac runs these against the page; GTK's reader runs the same rules.
    use postio_ui::reader::thread;
    assert_eq!(
        postio_ffi::thread_scroll_script("m-4".to_owned()),
        thread::scroll_script("m-4")
    );
    assert_eq!(
        postio_ffi::thread_toggle_script("m-4".to_owned()),
        thread::toggle_script("m-4")
    );
    assert_eq!(
        postio_ffi::thread_expand_all_script(),
        thread::expand_all_script()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_that_lost_a_part_says_so_on_its_anchor() {
    // The caveat is native chrome above the page, the way GTK's
    // `DecodeNotice` is -- so it crosses beside the page, per message, from
    // the body load the document already paid for (#1589).
    let database = test_support::memory().await;
    let thread = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let mut thread = Thread::new(account.id);
        let threads = ThreadRepository::new(&connection);
        threads.create(&mut thread).await.expect("a thread");
        let broken = message(&connection, account.id, inbox, "ada@example.com", 100, None).await;
        MessageRepository::new(&connection)
            .set_body(
                broken.id,
                &StoredBody {
                    text: Some("most of it".to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: true,
                },
                BodyState::Full,
            )
            .await
            .expect("a body");
        let fine = message(
            &connection,
            account.id,
            inbox,
            "bo@example.org",
            200,
            Some("<p>ok</p>"),
        )
        .await;
        for message in [&broken, &fine] {
            threads
                .add_message(thread.id, message.id)
                .await
                .expect("add");
        }
        thread.id.get()
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database)).expect("a session over the store");

    let document = session.thread_document(thread, Vec::new()).await;

    assert_eq!(
        document.messages[0].caveat.as_deref(),
        postio_ui::reader::document::decode_caveat(true)
    );
    assert_eq!(
        document.messages[1].caveat, None,
        "a whole message says nothing"
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_header_says_what_each_verb_will_act_on() {
    // FR-008 and FR-008a: the header's Reply goes to the latest message and
    // its Archive takes the whole thread, and the interface has to say so
    // before either is pressed. The words are `ReaderAction::describe`'s.
    let (session, _, _) = a_thread().await;
    let actions = session.conversation_actions(6);
    let reply = actions
        .iter()
        .find(|action| action.command == "reply")
        .expect("Reply");
    assert_eq!(reply.description, "Reply to the latest message");
    assert!(!reply.whole_conversation);
    let archive = actions
        .iter()
        .find(|action| action.command == "archive")
        .expect("Archive");
    assert_eq!(archive.description, "Archive all 6 messages");
    assert!(archive.whole_conversation);
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn the_rail_has_a_row_for_every_message_from_the_thread_itself() {
    // FR-040: a row the moment the conversation is known, not when a body
    // arrives -- the rail is for skipping a long message, and waiting for its
    // body would make you wait for exactly what you were skipping.
    let (session, thread, _) = a_thread().await;
    let document = session.thread_document(thread, Vec::new()).await;
    assert_eq!(
        document
            .rail
            .iter()
            .map(|row| row.position)
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
    assert_eq!(document.rail[0].sender, "ada@example.com");
    assert!(!document.rail[0].initials.is_empty());
    session.shutdown();
}
