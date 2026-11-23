use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::promoted::PromotedHeaders;
use postio_model::{BodyState, Generation, UidValidity};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use postio_sync::sync_mailbox;

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn mail(n: u32, from: &str, extra: &str, subject: &str) -> MockMessage {
    MockMessage::new(
        format!(
            "From: {from}\r\n\
             To: Test User <test@example.com>\r\n\
             Message-ID: <classify-{n}@example.com>\r\n\
             {extra}Subject: {subject}\r\n\r\nBody.\r\n"
        )
        .into_bytes(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_report_counts_what_the_classifier_did_and_names_no_mail() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(mail(
                    1,
                    "Ledger Weekly <news@ledger.example>",
                    "List-Unsubscribe: <https://ledger.example/u>\r\n",
                    "This week in the ledger",
                ))
                .message(mail(
                    2,
                    "Billing <no-reply@billing.example>",
                    "",
                    "Your receipt",
                ))
                .message(mail(
                    3,
                    "Ada Norwood <ada.norwood@example.org>",
                    "",
                    "The signed copy",
                )),
        )
        .build();
    backend.connect().await.expect("connect");
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("the sync");
    let messages = MessageRepository::new(&connection);
    for uid in messages
        .uids_in(inbox.id, Generation::new(VALIDITY))
        .await
        .expect("uids")
    {
        let row = messages
            .by_uid(inbox.id, Generation::new(VALIDITY), uid)
            .await
            .expect("a row")
            .expect("stored");
        let id = row.id;
        // What an arrival's header fetch records; a first sync's backlog is never asked.
        let offered = row.from.iter().any(|from| from.address.contains("ledger"));
        messages
            .set_promoted(
                id,
                PromotedHeaders {
                    unsubscribe_offered: offered,
                    automation: 0,
                },
            )
            .await
            .expect("the promoted headers");
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some("Could you send me the signed copy by Friday?".to_owned()),
                    ..StoredBody::default()
                },
                BodyState::Full,
            )
            .await
            .expect("a body");
    }

    let text = postio_session::diag::classify(connection, &postio_config::FocusConfig::default())
        .await
        .expect("the report");

    assert!(text.contains("examined 3\n"), "{text}");
    assert!(text.contains("filed 2\n"), "{text}");
    assert!(
        text.contains(" header 1:"),
        "the unsubscribe offer by its headers: {text}"
    );
    assert!(
        text.contains(" senders 1:"),
        "the no-reply sender by the table: {text}"
    );
    assert!(
        text.contains("considered 1:"),
        "only Ada's mail is asked: {text}"
    );
    assert!(
        text.contains("noticed 1:"),
        "the no-reply sender is read for a notice: {text}"
    );
    assert!(text.contains("  question detector 1:"), "{text}");
    for mail in [
        "Ledger",
        "ledger.example",
        "billing",
        "Ada",
        "example.org",
        "signed copy",
        "receipt",
    ] {
        assert!(!text.contains(mail), "{mail:?} reached the report: {text}");
    }
}
