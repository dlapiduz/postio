use postio_account::backend::{Fault, MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{AccountId, Flag, FlagSet, Mailbox, Uid, UidValidity};
use postio_storage::Checkout;
use postio_storage::Connection;
use postio_storage::repository::{ContactRepository, MessageRepository, SyncStateRepository};
use postio_storage::test_support;
use postio_sync::{Outcome, resync_mailbox, sync_mailbox, sync_mailbox_with_batch_size};

async fn times_ada_was_seen(connection: &Connection, _account_id: AccountId) -> u32 {
    ContactRepository::new(connection)
        .by_address("ada@example.com")
        .await
        .expect("look ada up")
        .map(|person| person.times_seen)
        .unwrap_or(0)
}

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn rid(uid: u32) -> postio_model::RemoteId {
    postio_model::RemoteId::new(format!("{VALIDITY}:{uid}"))
}

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

async fn server_with_messages(count: u32) -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");
    backend
}

async fn local(connection: &Connection) -> (AccountId, Mailbox) {
    let account = test_support::account(connection).await;
    let inbox = test_support::mailbox(connection, &account, INBOX).await;
    (account.id, inbox)
}

async fn bootstrap(connection: &Checkout, backend: &MockBackend, mailbox: &Mailbox) {
    sync_mailbox(connection, backend, mailbox, &CancelToken::new(), |_| {})
        .await
        .expect("bootstrap sync");
}

async fn known_uids(connection: &Connection, mailbox: &Mailbox) -> Vec<u32> {
    MessageRepository::new(connection)
        .uids_in(mailbox.id, postio_model::Generation::new(VALIDITY))
        .await
        .expect("uids_in")
        .into_iter()
        .map(Uid::get)
        .collect()
}

#[tokio::test]
async fn a_reconnect_with_no_server_changes_fetches_essentially_nothing() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let calls_before = backend.calls();
    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(outcome, Outcome::UpToDate);
    assert_eq!(backend.calls() - calls_before, 1);
    assert_eq!(known_uids(&connection, &inbox).await, vec![1, 2, 3]);
}

#[tokio::test]
async fn a_server_side_flag_change_and_deletion_both_reflect_locally() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend
        .store_flags(
            INBOX,
            &[rid(2)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");
    backend
        .store_flags(
            INBOX,
            &[rid(3)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend
        .expunge(INBOX, Some(&[rid(3)]))
        .await
        .expect("expunge");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Incremental {
            changed,
            vanished,
            arrived,
            ..
        } => {
            assert_eq!(changed, 1, "only message 2's flag change was reported");
            assert_eq!(vanished, 1, "message 3 is gone");
            assert!(
                arrived.is_empty(),
                "a flag change is not new mail: {arrived:?}"
            );
        }
        other => panic!("expected an incremental resync, got {other:?}"),
    }

    let messages = MessageRepository::new(&connection);
    let seen = messages
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(2),
        )
        .await
        .expect("look up message 2")
        .expect("message 2 still stored");
    assert!(
        seen.flags.is_seen(),
        "the flag change must reach the local row"
    );

    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2],
        "the expunged message must be gone locally too"
    );
}

#[tokio::test]
async fn a_deletion_that_leaves_highestmodseq_alone_still_reflects_locally() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend.expunge_keeps_mod_seq();
    backend
        .store_flags(
            INBOX,
            &[rid(3)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("settle the flag change");
    backend
        .expunge(INBOX, Some(&[rid(3)]))
        .await
        .expect("expunge");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert!(
        matches!(outcome, Outcome::Incremental { vanished: 1, .. }),
        "the deletion is the whole of the news and must be found: {outcome:?}"
    );
    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2],
        "the expunged message must be gone locally"
    );
}

#[tokio::test]
async fn a_message_the_client_cannot_decode_is_not_taken_for_expunged() {
    let backend = server_with_messages(4).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend.drop_from_header_fetches(2);
    backend
        .store_flags(
            INBOX,
            &[rid(4)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend
        .expunge(INBOX, Some(&[rid(4)]))
        .await
        .expect("expunge");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert!(
        matches!(outcome, Outcome::Incremental { vanished: 1, .. }),
        "only message 4 was expunged: {outcome:?}"
    );
    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2, 3],
        "message 2 is still on the server and must stay"
    );
}

#[tokio::test]
async fn one_expunge_in_a_large_folder_does_not_refetch_every_header() {
    const FOLDER: u32 = 300;
    let backend = server_with_messages(FOLDER).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend
        .store_flags(
            INBOX,
            &[rid(7)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend
        .expunge(INBOX, Some(&[rid(7)]))
        .await
        .expect("expunge");

    let rows_before = backend.header_fetch_rows().len();
    let listings_before = backend.uid_listings().len();
    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert!(
        matches!(outcome, Outcome::Incremental { vanished: 1, .. }),
        "{outcome:?}"
    );
    let rows = backend.header_fetch_rows();
    let largest = rows[rows_before..].iter().copied().max().unwrap_or(0);
    assert!(
        largest < 10,
        "the pass downloaded {largest} headers to learn that one message went"
    );
    assert_eq!(
        backend.uid_listings().len() - listings_before,
        1,
        "presence is one UID listing"
    );
    assert_eq!(known_uids(&connection, &inbox).await.len(), 299);
}

#[tokio::test]
async fn a_short_uid_listing_deletes_nothing() {
    let backend = server_with_messages(6).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend
        .store_flags(
            INBOX,
            &[rid(6)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend
        .expunge(INBOX, Some(&[rid(6)]))
        .await
        .expect("expunge");
    backend.truncate_uid_listing(2);

    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2, 3, 4, 5, 6],
        "a listing that disagrees with EXISTS is not evidence of anything"
    );
}

#[tokio::test]
async fn without_a_uid_listing_the_check_still_finds_the_expunge() {
    let backend = server_with_messages(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend.refuse_uid_listing();
    backend.drop_from_header_fetches(2);
    backend
        .store_flags(
            INBOX,
            &[rid(5)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
        )
        .await
        .expect("mark deleted");
    backend
        .expunge(INBOX, Some(&[rid(5)]))
        .await
        .expect("expunge");

    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2, 3, 4, 5],
        "two messages look absent but only one went: the answer is not trusted, nothing is deleted"
    );
}

#[tokio::test]
async fn a_flag_only_change_does_not_double_count_the_correspondent() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account_id, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;
    assert_eq!(
        times_ada_was_seen(&connection, account_id).await,
        3,
        "bootstrap already saw ada on all three messages"
    );

    backend
        .store_flags(
            INBOX,
            &[rid(2)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");
    assert!(matches!(outcome, Outcome::Incremental { changed: 1, .. }));

    assert_eq!(
        times_ada_was_seen(&connection, account_id).await,
        3,
        "a flag change on a message already seen must not count as a new sighting"
    );
}

#[tokio::test]
async fn a_message_the_change_feed_never_mentions_still_arrives() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend
        .append(INBOX, &postio_account::backend::AppendMessage::new(note(3)))
        .await
        .expect("deliver");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Incremental {
            changed,
            vanished,
            arrived,
            ..
        } => {
            assert_eq!(changed, 1, "UIDNEXT moved, so the gap was fetched");
            assert_eq!(vanished, 0);
            assert_eq!(
                arrived.len(),
                1,
                "the gap UIDNEXT caught is exactly one new message"
            );
        }
        other => panic!("expected an incremental resync, got {other:?}"),
    }
    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2, 3],
        "UIDNEXT is the second witness for an arrival, and it cannot be wrong \
         without the server being incoherent"
    );
}

#[tokio::test]
async fn an_arrival_during_resync_is_recorded_as_a_correspondent() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account_id, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;
    assert_eq!(times_ada_was_seen(&connection, account_id).await, 2);

    backend
        .append(INBOX, &postio_account::backend::AppendMessage::new(note(3)))
        .await
        .expect("deliver");

    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(
        times_ada_was_seen(&connection, account_id).await,
        3,
        "the incremental pull found a new message, so it must add a sighting"
    );
}

#[tokio::test]
async fn a_conforming_server_costs_no_extra_round_trip_for_arrivals() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend
        .store_flags(
            INBOX,
            &[rid(1)],
            &postio_account::backend::FlagChange::Add(FlagSet::from_iter([Flag::Seen])),
        )
        .await
        .expect("flag");

    let before = backend.calls();
    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert_eq!(
        backend.calls() - before,
        2,
        "one SELECT and one FETCH: when the change feed accounts for UIDNEXT \
         there is no gap left to ask about"
    );
}

#[tokio::test]
async fn a_uid_validity_change_wipes_and_rebuilds_the_mailbox() {
    let backend = server_with_messages(2).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let new_validity = UidValidity::new(1_900_000_000);
    backend.change_uid_validity(INBOX, new_validity);
    backend.acknowledge_uid_validity(INBOX);

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    match outcome {
        Outcome::Full { reason, report } => {
            assert_eq!(reason, postio_model::FullResyncReason::GenerationChanged);
            assert_eq!(report.inserted, 2);
        }
        other => panic!("expected a full resync, got {other:?}"),
    }

    let messages = MessageRepository::new(&connection);
    assert!(
        messages
            .by_uid(
                inbox.id,
                postio_model::Generation::new(VALIDITY),
                Uid::new(1)
            )
            .await
            .expect("look up under the old generation")
            .is_none(),
        "rows under the stale UIDVALIDITY must be gone"
    );
    let rebuilt = messages
        .by_uid(
            inbox.id,
            postio_model::Generation::new(new_validity.get()),
            Uid::new(1),
        )
        .await
        .expect("look up under the new generation")
        .expect("rebuilt under the new generation");
    assert_eq!(rebuilt.server.uid_validity, Some(new_validity));

    let state = SyncStateRepository::new(&connection)
        .require(inbox.id)
        .await
        .expect("sync state");
    assert_eq!(
        state.generation,
        Some(postio_model::Generation::new(new_validity.get()))
    );
    assert!(state.has_synced());
}

#[tokio::test]
async fn a_transient_backend_failure_during_resync_is_not_treated_as_a_resync_result() {
    let backend = server_with_messages(1).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    backend.inject(Fault::Disconnect);
    let error = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect_err("a dropped connection must surface as an error, not UpToDate");
    assert!(matches!(error, postio_sync::SyncError::Backend(_)));
}

async fn read_locally_and_enqueue(
    connection: &Connection,
    account: AccountId,
    message: postio_model::MessageId,
) {
    use postio_storage::repository::{FlagSource, OperationQueueRepository};
    let mut flags = FlagSet::new();
    flags.insert(Flag::Seen);
    MessageRepository::new(connection)
        .set_flags(message, &flags, FlagSource::Local)
        .await
        .expect("the local write");
    OperationQueueRepository::new(connection)
        .enqueue(
            account,
            postio_model::OperationTarget::Message(message),
            &postio_model::Operation::SetFlags { flags },
            chrono::Utc::now(),
        )
        .await
        .expect("enqueue");
}

#[tokio::test]
async fn a_read_that_has_not_drained_survives_the_resync_that_has_not_heard_it() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = local(&connection).await;
    let backend = server_with_messages(3).await;
    bootstrap(&connection, &backend, &inbox).await;

    let message = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(1),
        )
        .await
        .expect("read")
        .expect("the first message");
    assert!(
        !message.flags.contains(&Flag::Seen),
        "the fixture arrives unread, or this test is about nothing"
    );

    read_locally_and_enqueue(&connection, account, message.id).await;

    let mut flagged = FlagSet::new();
    flagged.insert(Flag::Flagged);
    backend
        .store_flags(
            INBOX,
            &[rid(1)],
            &postio_account::backend::FlagChange::Add(flagged),
        )
        .await
        .expect("the other client's STORE");

    resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    let after = MessageRepository::new(&connection)
        .get(message.id)
        .await
        .expect("read")
        .expect("the message");
    assert!(
        after.flags.contains(&Flag::Flagged),
        "the other client's flag never arrived, so this test is not watching \
         the pass it thinks it is"
    );
    assert!(
        after.flags.contains(&Flag::Seen),
        "the resync took the read back off: the row goes bold again, and the \
         queued operation will set a \\Seen on the server whose effect nobody \
         here can see (#317)"
    );
}

#[tokio::test]
async fn a_modseq_less_backend_resyncs_in_place_without_discarding_rows() {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=3u32 {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    let backend = MockBackend::builder()
        .capabilities(["IMAP4rev1"])
        .mailbox(mailbox)
        .build();
    backend.connect().await.expect("connect");

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;
    bootstrap(&connection, &backend, &inbox).await;

    let before: Vec<i64> = {
        let messages = MessageRepository::new(&connection);
        let uids = messages
            .uids_in(inbox.id, postio_model::Generation::new(VALIDITY))
            .await
            .expect("uids");
        let mut ids = Vec::with_capacity(uids.len());
        for uid in uids {
            ids.push(
                messages
                    .by_uid(inbox.id, postio_model::Generation::new(VALIDITY), uid)
                    .await
                    .expect("read")
                    .expect("the row")
                    .id
                    .get(),
            );
        }
        ids
    };
    assert_eq!(before.len(), 3, "the fixture synced");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");
    assert!(
        matches!(
            outcome,
            Outcome::Full {
                reason: postio_model::FullResyncReason::NoModSeq,
                ..
            }
        ),
        "the fixture is about the modseq-less plan: {outcome:?}"
    );

    let after: Vec<i64> = {
        let messages = MessageRepository::new(&connection);
        let uids = messages
            .uids_in(inbox.id, postio_model::Generation::new(VALIDITY))
            .await
            .expect("uids");
        let mut ids = Vec::with_capacity(uids.len());
        for uid in uids {
            ids.push(
                messages
                    .by_uid(inbox.id, postio_model::Generation::new(VALIDITY), uid)
                    .await
                    .expect("read")
                    .expect("the row")
                    .id
                    .get(),
            );
        }
        ids
    };
    assert_eq!(
        after, before,
        "the same local rows, not freshly inserted replacements: the \
         generation held, so nothing was wiped"
    );
}

#[tokio::test]
async fn a_pass_that_fetches_nothing_does_not_throw_away_what_the_last_one_stored() {
    let backend = server_with_messages(6).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    let cancel = CancelToken::new();
    let _ = sync_mailbox_with_batch_size(&connection, &backend, &inbox, 1, &cancel, |_| {
        cancel.cancel();
    })
    .await;
    let carried = known_uids(&connection, &inbox).await;
    assert!(
        !carried.is_empty() && carried.len() < 6,
        "this case needs a pass that got part of the way: it stored {carried:?}"
    );
    assert!(
        SyncStateRepository::new(&connection)
            .get(inbox.id)
            .await
            .expect("sync state")
            .is_none_or(|state| !state.has_synced()),
        "an interrupted pass must not have recorded a completed full sync"
    );

    let cancel = CancelToken::new();
    cancel.cancel();
    let _ = resync_mailbox(&connection, &backend, &inbox, &cancel, |_| {}).await;

    assert_eq!(
        known_uids(&connection, &inbox).await,
        carried,
        "the next pass wiped what the interrupted one had already stored, so \
         nothing accumulates and a mailbox too big to finish in one pass \
         never finishes"
    );
}

#[tokio::test]
async fn a_mailbox_holding_less_than_exists_re_enumerates() {
    let backend = server_with_messages(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    bootstrap(&connection, &backend, &inbox).await;
    let thinned = ids_for(&connection, &inbox, &[1, 2, 3, 4]).await;
    MessageRepository::new(&connection)
        .delete(&thinned)
        .await
        .expect("thin the mailbox out");
    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![5],
        "the fixture is the poisoned state: stamped synced, holding one of five"
    );

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert!(
        matches!(
            outcome,
            Outcome::Full {
                reason: postio_model::FullResyncReason::ShortOfExists,
                ..
            }
        ),
        "a mailbox short of EXISTS must re-enumerate rather than ask what \
         changed since a sync that never happened, got: {outcome:?}"
    );
    assert_eq!(
        known_uids(&connection, &inbox).await,
        vec![1, 2, 3, 4, 5],
        "the mail it was short of is fetched"
    );
}

#[tokio::test]
async fn a_gap_the_queue_explains_is_left_alone() {
    let backend = server_with_messages(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account, inbox) = local(&connection).await;

    bootstrap(&connection, &backend, &inbox).await;
    let archived = MessageRepository::new(&connection)
        .by_uid(
            inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(3),
        )
        .await
        .expect("by_uid")
        .expect("the message is there")
        .id;
    MessageRepository::new(&connection)
        .delete(&[archived])
        .await
        .expect("archive locally");
    postio_storage::repository::OperationQueueRepository::new(&connection)
        .enqueue(
            account,
            postio_model::OperationTarget::Message(archived),
            &postio_model::Operation::Move {
                from: inbox.id,
                to: postio_model::MailboxId::new(inbox.id.get() + 1),
            },
            chrono::Utc::now(),
        )
        .await
        .expect("enqueue the archive");

    let outcome = resync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("resync");

    assert!(
        !matches!(
            outcome,
            Outcome::Full {
                reason: postio_model::FullResyncReason::ShortOfExists,
                ..
            }
        ),
        "the queue explains the gap; re-enumerating would undo the archive"
    );
    assert!(
        !known_uids(&connection, &inbox).await.contains(&3),
        "the archived message must not come back"
    );
}

async fn ids_for(
    connection: &Connection,
    mailbox: &Mailbox,
    uids: &[u32],
) -> Vec<postio_model::MessageId> {
    let repository = MessageRepository::new(connection);
    let mut ids = Vec::new();
    for uid in uids {
        let message = repository
            .by_uid(
                mailbox.id,
                postio_model::Generation::new(VALIDITY),
                Uid::new(*uid),
            )
            .await
            .expect("by_uid")
            .expect("the message is there");
        ids.push(message.id);
    }
    ids
}
