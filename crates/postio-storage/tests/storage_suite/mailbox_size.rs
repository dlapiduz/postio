//! What an account's mail costs, before a byte of it is fetched — ADR 0017.

use postio_storage::repository::{MessageRepository, StorageFootprint};
use postio_storage::test_support;

/// A headers-only message, the state the footprint is computed from.
fn message(
    mailbox: postio_model::MailboxId,
    account: postio_model::AccountId,
    uid: u32,
) -> postio_model::Message {
    let mut message = postio_model::Message::new(
        account,
        mailbox,
        chrono::TimeZone::with_ymd_and_hms(&chrono::Utc, 2026, 3, 1, 9, 0, uid).unwrap(),
    );
    message.server.uid = Some(postio_model::Uid::new(uid));
    message.server.uid_validity = Some(postio_model::UidValidity::new(1));
    message.sync.body_state = postio_model::BodyState::HeadersOnly;
    message
}

#[test]
fn the_footprint_is_known_from_headers_alone() {
    // The nicest property of the whole measurement: `BODYSTRUCTURE` arrives
    // with the header sync, so `messages.size` and `attachments.size` are
    // populated for mail nobody has downloaded. Postio can say "1.4 GB of
    // mail, 11 GB of attachments" having spent nothing.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    // A message that is all words, and one that is mostly a PDF.
    let mut plain = message(inbox, account.id, 1);
    plain.size = 2_000;
    messages.create(&mut plain).expect("create");

    let mut with_payload = message(inbox, account.id, 2);
    with_payload.size = 1_000_000;
    with_payload.attachments = vec![{
        let mut attachment = postio_model::Attachment::new(
            postio_model::MessageId::UNASSIGNED,
            "application/pdf",
            990_000,
        );
        attachment.filename = Some("statement.pdf".to_owned());
        attachment.part_id = Some("2".to_owned());
        attachment
    }];
    messages.create(&mut with_payload).expect("create");

    let footprint = messages.footprint(account.id).expect("footprint");

    assert_eq!(footprint.messages, 2);
    assert_eq!(footprint.total_bytes, 1_002_000);
    assert_eq!(footprint.attachment_bytes, 990_000);
    // What the text axis will actually pull: everything that is not payload.
    assert_eq!(footprint.text_bytes(), 12_000);
    assert_eq!(footprint.local_bytes, 0, "nothing is downloaded yet");
}

#[test]
fn an_account_with_no_mail_has_an_empty_footprint() {
    // The empty state owes an answer too, and it must not be a division by
    // zero or a claim of "0 B of 0 B" that reads like a bug.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, _inbox) = test_support::account_with_inbox(&connection);

    let footprint = MessageRepository::new(&connection)
        .footprint(account.id)
        .expect("footprint");

    assert_eq!(footprint, StorageFootprint::default());
    assert!(footprint.is_empty());
}

#[test]
fn a_footprint_is_a_lower_bound_until_every_folder_has_synced_headers() {
    // The honesty this issue exists for. While the header pass is still
    // running the total climbs, and a number that grows every few seconds
    // looks broken -- so the surface must be able to say "over 1.4 GB" rather
    // than a total it is about to contradict.
    //
    // Complete means every selectable mailbox has a `last_full_sync_at`.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = message(inbox, account.id, 1);
    message.size = 2_000;
    messages.create(&mut message).expect("create");

    let footprint = messages.footprint(account.id).expect("footprint");
    assert!(
        !footprint.complete,
        "no folder has finished a header pass, so this is a floor"
    );

    connection
        .execute(
            "INSERT INTO sync_state (mailbox_id, account_id, uid_validity, last_full_sync_at)
             VALUES (?1, ?2, 1, 1000)
             ON CONFLICT (mailbox_id) DO UPDATE SET last_full_sync_at = 1000",
            rusqlite::params![inbox.get(), account.id.get()],
        )
        .expect("record a completed header pass");

    let footprint = messages.footprint(account.id).expect("footprint");
    assert!(footprint.complete, "now the total is a total");
}

#[test]
fn local_bytes_count_only_what_is_actually_downloaded() {
    // What the progress line divides by. A message whose text is local but
    // whose payload is not has contributed its text and not its payload --
    // which is the ordinary steady state under ADR 0017, not a special case.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut message = message(inbox, account.id, 1);
    message.size = 100_000;
    message.sync.body_state = postio_model::BodyState::Partial;
    message.attachments = vec![postio_model::Attachment::new(
        postio_model::MessageId::UNASSIGNED,
        "application/pdf",
        90_000,
    )];
    messages.create(&mut message).expect("create");

    let footprint = messages.footprint(account.id).expect("footprint");
    assert_eq!(
        footprint.local_bytes, 10_000,
        "its words are here, its attachment is not"
    );
}
