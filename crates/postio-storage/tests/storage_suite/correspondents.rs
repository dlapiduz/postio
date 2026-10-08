//! Who the person has written to (spec 007, data-model.md
//! "`correspondents`"): one row per address, counting the messages sent
//! with it in To, Cc or Bcc. What the filter's "you wrote to them" guard
//! (FR-111) and completion's "wrote N times" (FR-052) read.

use chrono::{DateTime, TimeZone, Utc};
use postio_model::{Account, EmailAddress, Identity, Message, MessageId};
use postio_storage::Connection;
use postio_storage::repository::{AccountRepository, CorrespondentRepository, MessageRepository};
use postio_storage::test_support;
use postio_storage::test_support::counting::{counted_async, scans};

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 25, hour, 0, 0).unwrap()
}

fn address(address: &str) -> EmailAddress {
    EmailAddress::new(None::<String>, address)
}

/// An account that sends as `ada@example.com` and, through a second
/// identity, as `Ada@Harbour.Example`, with a Sent folder.
async fn sender(connection: &Connection) -> (Account, postio_model::MailboxId) {
    let mut account = Account::new("Ada", address("ada@example.com"));
    let mut identity = Identity::new(
        postio_model::AccountId::UNASSIGNED,
        address("Ada@Harbour.Example"),
    );
    identity.is_default = true;
    account.identities = vec![identity];
    AccountRepository::new(connection)
        .create(&mut account)
        .await
        .expect("an account");
    let sent = test_support::mailbox(connection, &account, "Sent").await.id;
    (account, sent)
}

/// A message sent at `hour` to `to`, copying `cc`, blind-copying `bcc`.
async fn sent(
    connection: &Connection,
    (account, mailbox): (&Account, postio_model::MailboxId),
    hour: u32,
    to: &[&str],
    cc: &[&str],
    bcc: &[&str],
) -> MessageId {
    let mut message = Message::new(account.id, mailbox, at(hour));
    message.from = vec![account.address.clone()];
    message.to = to.iter().map(|to| address(to)).collect();
    message.cc = cc.iter().map(|cc| address(cc)).collect();
    message.bcc = bcc.iter().map(|bcc| address(bcc)).collect();
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("a sent message")
}

async fn written(connection: &Connection, to: &str) -> Option<(u32, Option<DateTime<Utc>>)> {
    CorrespondentRepository::new(connection)
        .get(&address(to))
        .await
        .expect("a read")
        .map(|correspondent| (correspondent.sent_count, correspondent.last_sent_at))
}

#[tokio::test]
async fn sending_to_three_addresses_adds_one_to_each() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, mailbox) = sender(&connection).await;
    let here = (&account, mailbox);
    // Three people -- one of them twice -- and the sender's own two
    // addresses, which are nobody the person "wrote to".
    let message = sent(
        &connection,
        here,
        9,
        &["grace@example.net", "tove@example.org"],
        &["quinn@example.com", "ADA@example.com"],
        &["grace@example.net", "ada@harbour.example"],
    )
    .await;
    let correspondents = CorrespondentRepository::new(&connection);
    let _ = correspondents.get(&address("warm@example.net")).await;

    let counts = counted_async(|| async {
        correspondents
            .record_sent(account.id, &[message])
            .await
            .expect("recorded");
    })
    .await;
    for to in ["grace@example.net", "tove@example.org", "quinn@example.com"] {
        assert_eq!(
            written(&connection, to).await,
            Some((1, Some(at(9)))),
            "{to}"
        );
    }
    for own in ["ada@example.com", "ada@harbour.example"] {
        assert_eq!(written(&connection, own).await, None, "{own} is the sender");
    }
    // The instrument counts reads: the sender's own addresses, once, and
    // nothing per recipient -- the counts are one write whatever the send.
    assert_eq!(counts.statements, 1, "{counts:?}");
}

#[tokio::test]
async fn each_message_counts_once_and_a_send_that_failed_takes_its_one_back() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, mailbox) = sender(&connection).await;
    let here = (&account, mailbox);
    let first = sent(&connection, here, 9, &["grace@example.net"], &[], &[]).await;
    let second = sent(
        &connection,
        here,
        11,
        &["grace@example.net"],
        &["tove@example.org"],
        &[],
    )
    .await;
    let correspondents = CorrespondentRepository::new(&connection);
    correspondents
        .record_sent(account.id, &[first, second])
        .await
        .expect("recorded");
    assert_eq!(
        written(&connection, "grace@example.net").await,
        Some((2, Some(at(11)))),
        "two messages, the later one last"
    );

    // The later send failed: nothing was delivered, so nothing was written.
    correspondents
        .unrecord_sent(account.id, second)
        .await
        .expect("taken back");
    assert_eq!(
        written(&connection, "grace@example.net").await.map(|w| w.0),
        Some(1)
    );
    assert_eq!(
        written(&connection, "tove@example.org").await.map(|w| w.0),
        Some(0)
    );
    assert!(
        !correspondents
            .wrote_to(&address("tove@example.org"))
            .await
            .expect("a read"),
        "a count of nothing is no one the person wrote to"
    );
    assert!(
        correspondents
            .wrote_to(&address("Grace@Example.NET"))
            .await
            .expect("a read"),
        "an address is who it is in any case"
    );
}

#[tokio::test]
async fn whether_the_person_wrote_to_someone_is_one_lookup() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, mailbox) = sender(&connection).await;
    let message = sent(
        &connection,
        (&account, mailbox),
        9,
        &["grace@example.net"],
        &[],
        &[],
    )
    .await;
    let correspondents = CorrespondentRepository::new(&connection);
    correspondents
        .record_sent(account.id, &[message])
        .await
        .expect("recorded");
    let _ = correspondents.wrote_to(&address("grace@example.net")).await;

    let mut wrote = false;
    let counts = counted_async(|| async {
        wrote = correspondents
            .wrote_to(&address("grace@example.net"))
            .await
            .expect("a read");
    })
    .await;
    assert!(wrote);
    assert_eq!((counts.statements, counts.rows), (1, 1), "{counts:?}");
    let sql = correspondents.explain_get();
    assert!(
        scans(&connection, &sql).await.is_empty(),
        "one seek by address, never a walk:\n{}",
        test_support::plan(&connection, &sql).await
    );
}

#[tokio::test]
async fn the_people_written_to_are_found_in_one_statement_and_counted() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, mailbox) = sender(&connection).await;
    let first = sent(
        &connection,
        (&account, mailbox),
        9,
        &["grace@example.com"],
        &["Linus@Example.Org"],
        &[],
    )
    .await;
    let second = sent(
        &connection,
        (&account, mailbox),
        10,
        &["grace@example.com"],
        &[],
        &[],
    )
    .await;
    let correspondents = CorrespondentRepository::new(&connection);
    correspondents
        .record_sent(account.id, &[first, second])
        .await
        .expect("counted");
    correspondents
        .record_sent(account.id, &[])
        .await
        .expect("nothing to count");

    let mut found = correspondents
        .written_to(&[
            address("GRACE@example.com"),
            address("linus@example.org"),
            address("stranger@example.net"),
        ])
        .await
        .expect("a read");
    found.sort();
    assert_eq!(
        found,
        vec![
            "grace@example.com".to_owned(),
            "linus@example.org".to_owned()
        ]
    );
    assert!(
        correspondents
            .written_to(&[])
            .await
            .expect("a read")
            .is_empty()
    );

    let counts = correspondents.sent_counts().await.expect("counts");
    assert_eq!(counts.len(), 2);
    assert_eq!(counts["grace@example.com"], 2);
    assert_eq!(counts["linus@example.org"], 1);

    // A send that failed takes its count back; zero is not "written to".
    correspondents
        .unrecord_sent(account.id, first)
        .await
        .expect("taken back");
    let counts = correspondents.sent_counts().await.expect("counts");
    assert_eq!(counts["grace@example.com"], 1);
    assert!(
        !counts.contains_key("linus@example.org"),
        "a count of zero is nobody"
    );
}
