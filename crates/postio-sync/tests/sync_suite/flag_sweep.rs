use postio_account::backend::{
    Fault, FlagChange, MailBackend, MockBackend, MockMailbox, MockMessage,
};
use postio_account::cancel::CancelToken;
use postio_model::{
    AccountId, Flag, FlagSet, Generation, Mailbox, ModSeq, SweepPolicy, Uid, UidValidity,
};
use postio_storage::repository::{MessageRepository, SyncStateRepository};
use postio_storage::test_support::{self, counting};
use postio_storage::{Checkout, Connection};
use postio_sync::{Outcome, resync_mailbox, resync_mailbox_with_policy, sync_mailbox};

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn rid(uid: u32) -> postio_model::RemoteId {
    postio_model::RemoteId::new(format!("{VALIDITY}:{uid}"))
}

fn flags(of: &[Flag]) -> FlagSet {
    of.iter().cloned().collect()
}

fn note(n: u32) -> Vec<u8> {
    format!(
        "From: Ada Lovelace <ada@example.com>\r\n\
         Subject: Note {n}\r\n\r\nBody {n}.\r\n"
    )
    .into_bytes()
}

fn mailbox_of(count: u32) -> MockMailbox {
    let mut mailbox = MockMailbox::new(INBOX).uid_validity(UidValidity::new(VALIDITY));
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(note(n)));
    }
    mailbox
}

async fn server(count: u32, unreliable: bool) -> MockBackend {
    let mut builder =
        MockBackend::builder().mailbox(mailbox_of(count).highest_mod_seq(ModSeq::new(100)));
    if unreliable {
        builder = builder.unreliable_change_feed();
    }
    let backend = builder.build();
    backend.connect().await.expect("connect");
    backend
}

async fn synced(count: u32, unreliable: bool) -> (MockBackend, Mailbox, AccountId, Fixture) {
    let backend = server(count, unreliable).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let account = test_support::account(&connection).await;
    let inbox = test_support::mailbox(&connection, &account, INBOX).await;
    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("first sync");
    (
        backend,
        inbox,
        account.id,
        Fixture {
            _database: database,
            connection,
        },
    )
}

struct Fixture {
    _database: postio_storage::Store,
    connection: Checkout,
}

async fn flags_of(connection: &Connection, mailbox: &Mailbox, uid: u32) -> FlagSet {
    MessageRepository::new(connection)
        .by_uid(mailbox.id, Generation::new(VALIDITY), Uid::new(uid))
        .await
        .expect("read")
        .expect("stored")
        .flags
}

async fn recorded_mod_seq(connection: &Connection, mailbox: &Mailbox) -> Option<ModSeq> {
    SyncStateRepository::new(connection)
        .require(mailbox.id)
        .await
        .expect("sync state")
        .highest_mod_seq
}

async fn resync(fixture: &Fixture, backend: &MockBackend, mailbox: &Mailbox) -> Outcome {
    resync_mailbox(
        &fixture.connection,
        backend,
        mailbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("resync")
}

#[tokio::test]
async fn a_change_pull_that_fails_leaves_highestmodseq_where_it_was() {
    let (backend, inbox, _, fixture) = synced(3, false).await;
    let before = recorded_mod_seq(&fixture.connection, &inbox).await;
    backend
        .store_flags(INBOX, &[rid(2)], &FlagChange::Add(flags(&[Flag::Seen])))
        .await
        .expect("another client reads message 2");

    backend.inject_after(1, Fault::Disconnect);
    resync_mailbox(
        &fixture.connection,
        &backend,
        &inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect_err("the pull was cut off");

    assert_eq!(
        recorded_mod_seq(&fixture.connection, &inbox).await,
        before,
        "recording the server's HIGHESTMODSEQ before the flags are in is how \
         a change becomes permanently missed"
    );
    assert!(!flags_of(&fixture.connection, &inbox, 2).await.is_seen());

    backend.clear_faults();
    backend.connect().await.expect("reconnect");
    resync(&fixture, &backend, &inbox).await;
    assert!(flags_of(&fixture.connection, &inbox, 2).await.is_seen());
}

#[tokio::test]
async fn a_rebuild_cut_short_after_the_modseq_went_backwards_is_not_up_to_date() {
    let (_, inbox, _, fixture) = synced(3, false).await;
    let rebuilt = MockBackend::builder()
        .mailbox(mailbox_of(3).highest_mod_seq(ModSeq::new(1)))
        .build();
    rebuilt.connect().await.expect("connect");
    let held = recorded_mod_seq(&fixture.connection, &inbox)
        .await
        .expect("a CONDSTORE server");
    assert!(held > ModSeq::new(1), "the fixture has to go backwards");

    rebuilt.inject_after(1, Fault::Disconnect);
    resync_mailbox(
        &fixture.connection,
        &rebuilt,
        &inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect_err("the re-read was cut off");

    assert_eq!(
        recorded_mod_seq(&fixture.connection, &inbox).await,
        Some(held),
        "the lower HIGHESTMODSEQ was recorded before the re-read finished"
    );

    rebuilt.clear_faults();
    rebuilt.connect().await.expect("reconnect");
    let outcome = resync(&fixture, &rebuilt, &inbox).await;
    assert!(
        matches!(outcome, Outcome::Full { .. }),
        "the interrupted re-read has to be done again, not skipped: {outcome:?}"
    );
    assert_eq!(
        recorded_mod_seq(&fixture.connection, &inbox).await,
        Some(ModSeq::new(1)),
        "a finished re-read records the server's value"
    );
}

#[tokio::test]
async fn a_reliable_server_is_believed_and_nothing_is_swept() {
    let (backend, inbox, _, fixture) = synced(3, false).await;
    backend.set_flags_silently(INBOX, Uid::new(2), flags(&[Flag::Seen]));

    let outcome = resync(&fixture, &backend, &inbox).await;

    assert_eq!(outcome, Outcome::UpToDate);
    assert!(backend.flag_fetches().is_empty(), "no sweep was asked for");
    assert!(
        !flags_of(&fixture.connection, &inbox, 2).await.is_seen(),
        "positive control: the change really is invisible to the change feed"
    );
}

#[tokio::test]
async fn an_unreliable_servers_silent_flag_change_is_found_by_the_sweep() {
    let (backend, inbox, _, fixture) = synced(3, true).await;
    let before = recorded_mod_seq(&fixture.connection, &inbox).await;
    backend.set_flags_silently(INBOX, Uid::new(2), flags(&[Flag::Seen, Flag::Flagged]));

    let outcome = resync(&fixture, &backend, &inbox).await;

    match outcome {
        Outcome::Incremental {
            changed,
            vanished,
            arrived,
            ..
        } => {
            assert_eq!(changed, 1, "one message's flags were corrected");
            assert_eq!(vanished, 0);
            assert!(arrived.is_empty(), "a flag change is not new mail");
        }
        other => panic!("a corrected flag is a change the list must repaint: {other:?}"),
    }
    assert_eq!(
        flags_of(&fixture.connection, &inbox, 2).await,
        flags(&[Flag::Seen, Flag::Flagged])
    );
    assert_eq!(
        flags_of(&fixture.connection, &inbox, 1).await,
        FlagSet::new(),
        "the sweep touched only what disagreed"
    );
    assert_eq!(backend.flag_fetches(), vec![INBOX.to_owned()]);
    assert_eq!(
        recorded_mod_seq(&fixture.connection, &inbox).await,
        before,
        "the server never moved its counter, and the sweep invents none"
    );

    assert_eq!(
        resync(&fixture, &backend, &inbox).await,
        Outcome::UpToDate,
        "a sweep that found nothing is not a change"
    );
}

#[tokio::test]
async fn a_sweep_also_runs_beside_a_change_pull_that_found_something() {
    let (backend, inbox, _, fixture) = synced(3, true).await;
    backend
        .store_flags(INBOX, &[rid(1)], &FlagChange::Add(flags(&[Flag::Flagged])))
        .await
        .expect("reported change");
    backend.set_flags_silently(INBOX, Uid::new(3), flags(&[Flag::Seen]));

    let outcome = resync(&fixture, &backend, &inbox).await;

    match outcome {
        Outcome::Incremental { changed, .. } => assert_eq!(changed, 2),
        other => panic!("expected an incremental pass: {other:?}"),
    }
    assert!(flags_of(&fixture.connection, &inbox, 1).await.is_flagged());
    assert!(flags_of(&fixture.connection, &inbox, 3).await.is_seen());
}

#[tokio::test]
async fn a_larger_folder_is_swept_only_every_nth_pass() {
    let (backend, inbox, _, fixture) = synced(5, true).await;
    let policy = SweepPolicy {
        every_pass_up_to: 4,
        larger_every: 3,
    };
    backend.set_flags_silently(INBOX, Uid::new(4), flags(&[Flag::Seen]));

    let mut seen_on = Vec::new();
    for pass in 1..=4 {
        resync_mailbox_with_policy(
            &fixture.connection,
            &backend,
            &inbox,
            None,
            policy,
            &CancelToken::new(),
            |_| {},
        )
        .await
        .expect("resync");
        if flags_of(&fixture.connection, &inbox, 4).await.is_seen() {
            seen_on.push(pass);
            break;
        }
    }

    assert_eq!(
        seen_on,
        vec![3],
        "two passes trust the feed and the third sweeps"
    );
    assert_eq!(backend.flag_fetches().len(), 1);
    let state = SyncStateRepository::new(&fixture.connection)
        .require(inbox.id)
        .await
        .expect("state");
    assert_eq!(
        state.passes_since_flag_sweep, 0,
        "a sweep restarts the count, and it survives in the store"
    );
}

#[tokio::test]
async fn passes_that_do_not_sweep_are_counted_in_the_store() {
    let (backend, inbox, _, fixture) = synced(5, true).await;
    let policy = SweepPolicy {
        every_pass_up_to: 4,
        larger_every: 10,
    };
    for _ in 0..2 {
        resync_mailbox_with_policy(
            &fixture.connection,
            &backend,
            &inbox,
            None,
            policy,
            &CancelToken::new(),
            |_| {},
        )
        .await
        .expect("resync");
    }

    let state = SyncStateRepository::new(&fixture.connection)
        .require(inbox.id)
        .await
        .expect("state");
    assert_eq!(state.passes_since_flag_sweep, 2);
    assert!(backend.flag_fetches().is_empty());
}

#[tokio::test]
async fn a_sweep_that_fails_leaves_highestmodseq_where_it_was() {
    let (backend, inbox, _, fixture) = synced(3, true).await;
    let before = recorded_mod_seq(&fixture.connection, &inbox).await;
    backend
        .store_flags(INBOX, &[rid(1)], &FlagChange::Add(flags(&[Flag::Flagged])))
        .await
        .expect("a reported change moves the counter");
    backend.set_flags_silently(INBOX, Uid::new(3), flags(&[Flag::Seen]));

    backend.inject_after(2, Fault::Disconnect);
    resync_mailbox(
        &fixture.connection,
        &backend,
        &inbox,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect_err("the sweep was cut off");

    assert_eq!(
        recorded_mod_seq(&fixture.connection, &inbox).await,
        before,
        "the counter moved past a pass whose flags were not reconciled"
    );

    backend.clear_faults();
    backend.connect().await.expect("reconnect");
    resync(&fixture, &backend, &inbox).await;
    assert!(flags_of(&fixture.connection, &inbox, 3).await.is_seen());
    assert_ne!(recorded_mod_seq(&fixture.connection, &inbox).await, before);
}

#[tokio::test]
async fn a_read_that_has_not_drained_survives_a_sweep() {
    use postio_storage::repository::{FlagSource, OperationQueueRepository};

    let (backend, inbox, account, fixture) = synced(3, true).await;
    let message = MessageRepository::new(&fixture.connection)
        .by_uid(inbox.id, Generation::new(VALIDITY), Uid::new(1))
        .await
        .expect("read")
        .expect("stored");

    let seen = flags(&[Flag::Seen]);
    MessageRepository::new(&fixture.connection)
        .set_flags(message.id, &seen, FlagSource::Local)
        .await
        .expect("local write");
    OperationQueueRepository::new(&fixture.connection)
        .enqueue(
            account,
            postio_model::OperationTarget::Message(message.id),
            &postio_model::Operation::SetFlags { flags: seen },
            chrono::Utc::now(),
        )
        .await
        .expect("enqueue");

    let outcome = resync(&fixture, &backend, &inbox).await;

    assert_eq!(backend.flag_fetches().len(), 1, "the sweep ran");
    assert!(
        flags_of(&fixture.connection, &inbox, 1).await.is_seen(),
        "the sweep took a read the server has not heard of back off the row (#317)"
    );
    assert_eq!(
        outcome,
        Outcome::UpToDate,
        "agreeing with the user's own pending change is not a correction"
    );
}

#[tokio::test]
async fn a_sweep_reads_the_store_once_and_writes_only_what_differs() {
    let (backend, inbox, _, fixture) = synced(50, true).await;
    backend.set_flags_silently(INBOX, Uid::new(7), flags(&[Flag::Seen]));
    backend.set_flags_silently(INBOX, Uid::new(8), flags(&[Flag::Flagged]));

    counting::record();
    counting::reset();
    let outcome = resync(&fixture, &backend, &inbox).await;
    let issued = counting::recorded();

    assert!(matches!(outcome, Outcome::Incremental { changed: 2, .. }));
    let count_of = |needle: &str| -> usize {
        issued
            .iter()
            .filter(|(sql, _)| sql.contains(needle))
            .map(|(_, n)| *n)
            .sum()
    };
    assert_eq!(
        count_of("SELECT id, uid, flags FROM messages"),
        1,
        "one listing of the store however many messages it holds: {issued:#?}"
    );
    assert_eq!(
        count_of("SELECT flags FROM messages WHERE id"),
        2,
        "a message is looked at again only if it disagreed: {issued:#?}"
    );
    assert_eq!(
        count_of("SET flags = ?2"),
        2,
        "two disagreements, two writes: {issued:#?}"
    );
}
