//! No test here configures `[focus.model]`, so nothing tries to reach a model, loopback included.

use std::sync::Arc;

use chrono::Utc;
use postio_ffi::{Session, SessionOptions, UiEvent};
use postio_model::Message;
use postio_storage::repository::{DigestRepository, MessageRepository, StoredBody};
use postio_storage::test_support;

use crate::focus::heard;

const LEDGER: &str = "The council voted 7 to 2 to fund the rail link. Work starts in March.";
const FORGE: &str = "Build 12 passed on every platform.";

async fn a_delivered_digest(config: &str) -> (Arc<Session>, i64, [i64; 2]) {
    let database = test_support::memory().await;
    let (delivery, ids) = {
        let connection = database.connect().await.expect("a connection");
        let (account, inbox) = test_support::account_with_inbox(&connection).await;
        let messages = MessageRepository::new(&connection);
        let at = Utc::now() - chrono::TimeDelta::hours(2);
        let mut ids = [0_i64; 2];
        for (slot, (name, address, subject, text)) in [
            (
                "The Ledger",
                "news@ledger.example",
                "Tonight's vote",
                LEDGER,
            ),
            ("Forge", "builds@forge.example", "Build 12", FORGE),
        ]
        .into_iter()
        .enumerate()
        {
            let mut message = Message::new(account.id, inbox, at);
            message.from = vec![postio_model::EmailAddress::new(Some(name), address)];
            message.subject = Some(subject.to_owned());
            message.date = Some(at);
            let id = messages.create(&mut message).await.expect("a message");
            messages
                .set_body(
                    id,
                    &StoredBody {
                        text: Some(text.to_owned()),
                        html: None,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    postio_model::message::BodyState::Full,
                )
                .await
                .expect("the body is stored");
            ids[slot] = id.get();
        }
        let digests = DigestRepository::new(&connection);
        for id in ids {
            digests
                .hold(postio_model::MessageId::new(id), "Newsletters", at)
                .await
                .expect("held");
        }
        let due = Utc::now() - chrono::TimeDelta::hours(1);
        let delivery = digests
            .deliver("Newsletters", due, due)
            .await
            .expect("delivered")
            .expect("a delivery");
        let summary = format!(
            r#"{{"statements":[
                {{"topic":"Rail","text":"The rail link is funded.","reference":{{"number":1,"message":{},"excerpt":"The council voted 7 to 2 to fund the rail link."}}}},
                {{"topic":"Builds","text":"Build 12 is green.","reference":{{"number":2,"message":{},"excerpt":"Build 12 passed on every platform."}}}}
            ],"messages":2,"senders":2}}"#,
            ids[0], ids[1]
        );
        digests
            .set_summary(delivery, &summary, due)
            .await
            .expect("the summary is written");
        (delivery.get(), ids)
    };
    let session =
        Session::open(SessionOptions::in_memory_with(database).with_config_for_test(config))
            .expect("a session");
    (session, delivery, ids)
}

const NEWSLETTERS: &str = "[[focus.digests]]\nname = \"Newsletters\"\n\
    match = [\"from:news@ledger.example\", \"from:builds@forge.example\"]\n\
    cadence = \"daily\"\nat = \"08:00\"\n";

#[tokio::test(flavor = "multi_thread")]
async fn a_digests_summary_crosses_with_its_numbered_references() {
    let (session, delivery, [ledger, forge]) = a_delivered_digest(NEWSLETTERS).await;
    let summary = session
        .digest_summary(delivery)
        .expect("read")
        .expect("a summary is written");
    let cited: Vec<(u32, i64)> = summary
        .statements
        .iter()
        .map(|statement| (statement.number, statement.message))
        .collect();
    assert_eq!(cited, [(1, ledger), (2, forge)]);
    assert_eq!(summary.statements[0].topic, "Rail");
    assert_eq!(
        summary.statements[0].excerpt,
        "The council voted 7 to 2 to fund the rail link."
    );
    session.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn with_no_vault_the_vault_read_fails_naming_where_to_configure_one() {
    let session = Session::open(SessionOptions::in_memory()).expect("a session");
    match session.vault("Quarterly review".to_owned()) {
        Err(postio_ffi::SessionError::StoreUnavailable { message }) => {
            assert!(message.contains("[focus.vault]"), "{message}");
        }
        other => panic!("no vault, no picture: {other:?}"),
    }
    session.shutdown();
}

fn vault_at(vault: &std::path::Path) -> String {
    format!("[focus.vault]\npath = {:?}\n", vault.display().to_string())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_captured_task_is_the_exact_line_with_the_link_before_the_date() {
    let vault = tempfile::tempdir().expect("a vault");
    let session =
        Session::open(SessionOptions::in_memory().with_config_for_test(&vault_at(vault.path())))
            .expect("a session");
    let captured = session
        .capture_task(
            None,
            "Send the slides".to_owned(),
            42,
            Some("2026-10-02".to_owned()),
        )
        .expect("written");
    let link = postio_ffi::message_link(42);
    let at_link = captured.line.find(&link).expect("the link");
    let at_date = captured.line.find("2026-10-02").expect("the date");
    assert!(at_link < at_date, "C21: {}", captured.line);
    assert!(captured.line.starts_with("- [ ] Send the slides "));
    let written = std::fs::read_to_string(vault.path().join(&captured.note)).expect("the note");
    assert!(
        written.lines().any(|line| line == captured.line),
        "the line written is the line said: {written:?}"
    );
    session.shutdown();
}

#[test]
fn a_message_link_names_its_message_and_nothing_else_does() {
    assert_eq!(postio_ffi::message_link(42), "postio://message/42");
    assert_eq!(
        postio_ffi::parse_message_link("postio://message/42/".to_owned()),
        Some(42)
    );
    assert_eq!(
        postio_ffi::parse_message_link("postio://message/0".to_owned()),
        None
    );
    assert_eq!(
        postio_ffi::parse_message_link("https://example.com/message/42".to_owned()),
        None
    );
    assert_eq!(postio_ffi::link_unknown(), postio_ui::links::UNKNOWN);
    assert_eq!(postio_ffi::link_gone(), postio_ui::links::GONE);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_postio_link_opens_its_message_or_says_why_not() {
    let session = crate::focus::inbox_of(&["Quarterly review"]).await;
    crate::focus::cursor_on_the_first_row(&session).await;
    let message = session.focus_row_at(0).expect("the row").id;
    session.focus_open_link(postio_ffi::message_link(message));
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusOpenMessage { message: opened, .. } if *opened == message
        ))
        .await,
        "the link opens its message"
    );
    session.focus_open_link("postio://message/9999".to_owned());
    assert!(
        heard(&session, 10, |event| matches!(
            event,
            UiEvent::FocusToast { text, .. } if text == postio_ui::links::GONE
        ))
        .await,
        "a message not here is said"
    );
    session.shutdown();
}
