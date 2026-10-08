//! Who the person wrote to, as a Sent folder syncs (spec 007, T075): every
//! message the person sent counts once, whether it was filed by this
//! client's send or brought by a sync of Sent -- and a sent copy the sync
//! adopts is not counted a second time.

use postio_account::backend::{AppendMessage, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{EmailAddress, Mailbox, RfcMessageId, UidValidity};
use postio_storage::Connection;
use postio_storage::repository::{CorrespondentRepository, MessageRepository};
use postio_storage::test_support;
use postio_sync::{resync_mailbox, sync_mailbox};

const SENT: &str = "Sent";
const VALIDITY: u32 = 1_707_000_000;

/// A message the account (`test@example.com`) sent.
fn sent_mail(n: u32, to: &str) -> Vec<u8> {
    format!(
        "From: Test User <test@example.com>\r\n\
         To: {to}\r\n\
         Message-ID: <sent-{n}@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

async fn sent_to(connection: &Connection, address: &str) -> Option<u32> {
    CorrespondentRepository::new(connection)
        .get(&EmailAddress::new(None::<String>, address))
        .await
        .expect("a read")
        .map(|correspondent| correspondent.sent_count)
}

async fn local_sent(connection: &Connection) -> (postio_model::Account, Mailbox) {
    let account = test_support::account(connection).await;
    let sent = test_support::mailbox(connection, &account, SENT).await;
    (account, sent)
}

#[tokio::test]
async fn a_sent_folder_s_sync_counts_each_message_it_files_once() {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(SENT)
                .uid_validity(UidValidity::new(VALIDITY))
                .message(MockMessage::new(sent_mail(1, "grace@example.net")))
                .message(MockMessage::new(sent_mail(
                    2,
                    "Grace <grace@example.net>, tove@example.org",
                ))),
        )
        .build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, sent) = local_sent(&connection).await;

    sync_mailbox(&connection, &backend, &sent, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync");
    assert_eq!(sent_to(&connection, "grace@example.net").await, Some(2));
    assert_eq!(sent_to(&connection, "tove@example.org").await, Some(1));
    assert_eq!(
        sent_to(&connection, "test@example.com").await,
        None,
        "the account's own address"
    );

    // A message sent from another client arrives; the pass that brings it
    // counts it, and one that brings nothing counts nothing again.
    backend
        .append(SENT, &AppendMessage::new(sent_mail(3, "tove@example.org")))
        .await
        .expect("sent elsewhere");
    resync_mailbox(&connection, &backend, &sent, &CancelToken::new(), |_| {})
        .await
        .expect("an incremental pass");
    resync_mailbox(&connection, &backend, &sent, &CancelToken::new(), |_| {})
        .await
        .expect("a pass with nothing new");
    assert_eq!(sent_to(&connection, "tove@example.org").await, Some(2));
    assert_eq!(sent_to(&connection, "grace@example.net").await, Some(2));
}

#[tokio::test]
async fn a_sent_copy_a_sync_adopts_is_not_counted_twice() {
    let backend = MockBackend::builder()
        .mailbox(MockMailbox::new(SENT).uid_validity(UidValidity::new(VALIDITY)))
        .build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, sent) = local_sent(&connection).await;
    sync_mailbox(&connection, &backend, &sent, &CancelToken::new(), |_| {})
        .await
        .expect("the first sync of an empty Sent");

    // This client sent it: the copy is filed and counted before the server
    // has named it, as `postio_sync::send` does, and the append that
    // followed came back without the server's UID for it.
    let raw = sent_mail(7, "quinn@example.com");
    let mut copy =
        postio_model::mime::parse(&raw).into_message(account.id, sent.id, chrono::Utc::now());
    copy.rfc_message_id = Some(RfcMessageId::new("<sent-7@example.com>"));
    let copy_id = MessageRepository::new(&connection)
        .create(&mut copy)
        .await
        .expect("the local copy");
    CorrespondentRepository::new(&connection)
        .record_sent(account.id, &[copy_id])
        .await
        .expect("counted at send");
    backend
        .append(SENT, &AppendMessage::new(raw))
        .await
        .expect("the append");

    // The next pass over Sent finds the server's copy and adopts the row by
    // its Message-ID rather than filing a second one.
    resync_mailbox(&connection, &backend, &sent, &CancelToken::new(), |_| {})
        .await
        .expect("a pass over Sent");
    let rows: i64 = postio_storage::sql::scalar(
        &connection,
        "SELECT count(*) FROM messages WHERE mailbox_id = ?1",
        [sent.id.get()],
    )
    .await
    .expect("a count");
    assert_eq!(rows, 1, "the copy was adopted, not filed twice");
    assert_eq!(
        sent_to(&connection, "quinn@example.com").await,
        Some(1),
        "one message, counted once"
    );
}
