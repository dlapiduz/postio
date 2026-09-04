//! `read_receipt_requested` round-trips through storage and the pane's count
//! reflects it (#970).
//!
//! Postio never sends a read receipt automatically (CLAUDE.md's privacy
//! section) — this is only the record of how often one was asked, the same
//! denormalize-at-ingest shape `messages.list_id` already uses.

use postio_model::AccountId;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

const ASKED: &[u8] = b"From: Newsletter <news@example.org>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: Please confirm receipt\r\n\
Disposition-Notification-To: news@example.org\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Let us know you got this\r\n";

const NOT_ASKED: &[u8] = b"From: Grace Hopper <grace@example.net>\r\n\
To: Ada Lovelace <ada@example.com>\r\n\
Subject: Ordinary mail\r\n\
MIME-Version: 1.0\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Nothing special\r\n";

#[test]
fn a_requested_receipt_survives_a_round_trip_through_storage() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    let repository = MessageRepository::new(&connection);

    let mut asked =
        postio_model::mime::parse(ASKED).into_message(account.id, mailbox, chrono::Utc::now());
    let id = repository.create(&mut asked).expect("create");

    let reloaded = repository
        .get(id)
        .expect("read")
        .expect("the message is there");
    assert!(
        reloaded.read_receipt_requested,
        "the header was present at ingest, so the stored row should say so"
    );
}

#[test]
fn a_message_that_never_asked_stores_false() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    let repository = MessageRepository::new(&connection);

    let mut plain =
        postio_model::mime::parse(NOT_ASKED).into_message(account.id, mailbox, chrono::Utc::now());
    let id = repository.create(&mut plain).expect("create");

    let reloaded = repository
        .get(id)
        .expect("read")
        .expect("the message is there");
    assert!(!reloaded.read_receipt_requested);
}

#[test]
fn the_count_is_scoped_to_its_own_account_and_ignores_ones_that_never_asked() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (mine, mine_inbox) = test_support::account_with_inbox(&connection);
    let repository = MessageRepository::new(&connection);

    let mut theirs_owner = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    postio_storage::repository::AccountRepository::new(&connection)
        .create(&mut theirs_owner)
        .expect("second account");
    let theirs_inbox = test_support::mailbox(&connection, &theirs_owner, "INBOX").id;

    for _ in 0..2 {
        let mut asked =
            postio_model::mime::parse(ASKED).into_message(mine.id, mine_inbox, chrono::Utc::now());
        repository.create(&mut asked).expect("create");
    }
    let mut plain =
        postio_model::mime::parse(NOT_ASKED).into_message(mine.id, mine_inbox, chrono::Utc::now());
    repository.create(&mut plain).expect("create");
    let mut theirs = postio_model::mime::parse(ASKED).into_message(
        theirs_owner.id,
        theirs_inbox,
        chrono::Utc::now(),
    );
    repository.create(&mut theirs).expect("create");

    assert_eq!(
        repository
            .read_receipt_requested_count(mine.id)
            .expect("count"),
        2,
        "two of mine asked, one did not, and the other account's should not count"
    );
    assert_eq!(
        repository
            .read_receipt_requested_count(AccountId::new(999_999))
            .expect("count"),
        0,
        "an account with no mail at all has nothing to count"
    );
}
