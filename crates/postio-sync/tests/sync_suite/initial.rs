use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::{AccountId, Mailbox, MailboxId, Uid};
use postio_storage::Connection;
use postio_storage::repository::{ContactRepository, MessageRepository, SyncStateRepository};
use postio_storage::test_support;
use postio_sync::{Progress, sync_mailbox, sync_mailbox_with_batch_size};
use std::collections::BTreeSet;

const INBOX: &str = "INBOX";

async fn server_with_messages(count: u32) -> MockBackend {
    let mut mailbox = MockMailbox::new(INBOX);
    for n in 1..=count {
        mailbox = mailbox.message(MockMessage::new(
            format!(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 Subject: Note {n}\r\n\r\nBody {n}.\r\n"
            )
            .into_bytes(),
        ));
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

fn uids_at_checkpoint(
    probe: &Connection,
    mailbox_id: MailboxId,
    generation: postio_model::Generation,
) -> BTreeSet<u32> {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(known_uids(probe, mailbox_id, generation))
    })
}

async fn known_uids(
    connection: &Connection,
    mailbox_id: MailboxId,
    generation: postio_model::Generation,
) -> BTreeSet<u32> {
    MessageRepository::new(connection)
        .uids_in(mailbox_id, generation)
        .await
        .expect("uids_in")
        .into_iter()
        .map(Uid::get)
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_newest_batch_lands_before_the_oldest_one() {
    let backend = server_with_messages(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let probe = database.connect().await.expect("a probe connection");
    let (_account, inbox) = local(&connection).await;

    let mut batches: Vec<BTreeSet<u32>> = Vec::new();
    let uid_validity = postio_model::Generation::new(1);
    sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        2,
        &CancelToken::new(),
        |_progress: Progress| {
            batches.push(uids_at_checkpoint(&probe, inbox.id, uid_validity));
        },
    )
    .await
    .expect("initial sync");

    assert_eq!(
        batches[0],
        BTreeSet::from([4, 5]),
        "the first commit must hold only the newest UIDs, not the oldest ones"
    );
    assert_eq!(batches.last(), Some(&BTreeSet::from([1, 2, 3, 4, 5])));
}

#[tokio::test]
async fn interrupting_and_restarting_does_not_refetch_completed_ranges() {
    let backend = server_with_messages(5).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    let cancel = CancelToken::new();
    let mut batches_seen = 0;
    let first_pass = sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        2,
        &cancel,
        |_progress: Progress| {
            batches_seen += 1;
            if batches_seen == 1 {
                cancel.cancel();
            }
        },
    )
    .await;

    assert!(
        first_pass.is_err(),
        "a cancelled pass must not silently succeed"
    );
    assert_eq!(
        known_uids(&connection, inbox.id, postio_model::Generation::new(1)).await,
        BTreeSet::from([4, 5]),
        "only the first batch should have committed"
    );
    assert!(
        !SyncStateRepository::new(&connection)
            .require(inbox.id)
            .await
            .expect("sync state")
            .has_synced(),
        "an interrupted pass must not be marked complete"
    );

    let resumed = sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        2,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("resumed sync");

    assert_eq!(resumed.inserted, 3);
    assert_eq!(resumed.updated, 0);
    assert_eq!(
        known_uids(&connection, inbox.id, postio_model::Generation::new(1)).await,
        BTreeSet::from([1, 2, 3, 4, 5])
    );
    assert!(
        SyncStateRepository::new(&connection)
            .require(inbox.id)
            .await
            .expect("sync state")
            .has_synced()
    );
}

#[tokio::test]
async fn a_reply_arriving_before_its_parent_still_finds_its_thread() {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                .message(MockMessage::new(
                    b"From: Ada Lovelace <ada@example.com>\r\n\
                      Subject: Analytical engine\r\n\
                      Message-ID: <parent@example.com>\r\n\r\nNotes.\r\n"
                        .to_vec(),
                ))
                .message(MockMessage::new(
                    b"From: Ada Lovelace <ada@example.com>\r\n\
                      Subject: Re: Analytical engine\r\n\
                      Message-ID: <reply@example.com>\r\n\
                      In-Reply-To: <parent@example.com>\r\n\
                      References: <parent@example.com>\r\n\r\nAgreed.\r\n"
                        .to_vec(),
                )),
        )
        .build();
    backend.connect().await.expect("connect");

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("initial sync");

    let messages = MessageRepository::new(&connection);
    let parent = messages
        .by_uid(inbox.id, postio_model::Generation::new(1), Uid::new(1))
        .await
        .expect("look up parent")
        .expect("parent stored");
    let reply = messages
        .by_uid(inbox.id, postio_model::Generation::new(1), Uid::new(2))
        .await
        .expect("look up reply")
        .expect("reply stored");

    assert!(parent.thread_id.is_some());
    assert_eq!(
        parent.thread_id, reply.thread_id,
        "the reply's thread must have claimed the parent once it arrived, \
         even though the reply was threaded first"
    );
}

#[tokio::test]
async fn a_full_sync_records_every_correspondent_as_a_contact() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (account_id, inbox) = local(&connection).await;

    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("initial sync");

    let _ = account_id;
    let people = ContactRepository::new(&connection)
        .people(10_000)
        .await
        .expect("list people");
    let ada = people
        .iter()
        .flat_map(|person| &person.addresses)
        .find(|owned| owned.address.normalized() == "ada@example.com")
        .expect("the sender of all three messages must be a recorded contact");
    assert_eq!(
        ada.times_seen, 3,
        "one sighting per message, not per address occurrence"
    );
}

#[tokio::test]
async fn progress_is_a_fraction_of_the_mail_not_of_the_uid_space() {
    let mut mailbox = MockMailbox::new(INBOX).starting_uid(1_000);
    for n in 1..=5 {
        mailbox = mailbox.message(MockMessage::new(
            format!(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 Subject: Note {n}\r\n\r\nBody {n}.\r\n"
            )
            .into_bytes(),
        ));
    }
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account_id, inbox) = local(&connection).await;

    let mut reports: Vec<Progress> = Vec::new();
    sync_mailbox(
        &connection,
        &backend,
        &inbox,
        &CancelToken::new(),
        |progress| reports.push(progress),
    )
    .await
    .expect("initial sync");

    let last = reports
        .last()
        .expect("a pass that wrote mail reports on it");
    assert_eq!(
        last.target, 5,
        "the folder holds five messages in a UID space over a thousand wide; \
         a denominator of {} is the UID ceiling, which renders as 0%",
        last.target
    );
    assert!(
        last.fetched >= last.target,
        "a pass that fetched everything there is reported {}/{} — so nothing \
         downstream can ever tell that it finished",
        last.fetched,
        last.target
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_next_batch_is_asked_for_before_this_one_is_committed() {
    let backend = server_with_messages(6).await;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    let mut served_at_commit: Vec<usize> = Vec::new();
    sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        2,
        &CancelToken::new(),
        |_progress: Progress| served_at_commit.push(backend.header_fetches().len()),
    )
    .await
    .expect("initial sync");

    assert_eq!(
        served_at_commit,
        vec![2, 3, 3],
        "each batch should be requested while its predecessor is being written"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn running_ahead_never_reorders_or_loses_a_batch() {
    let backend = server_with_messages(7).await;
    backend.set_latency(std::time::Duration::from_millis(5));
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let probe = database.connect().await.expect("a probe connection");
    let (_account, inbox) = local(&connection).await;

    let uid_validity = postio_model::Generation::new(1);
    let mut batches: Vec<BTreeSet<u32>> = Vec::new();
    let report = sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        3,
        &CancelToken::new(),
        |_progress: Progress| batches.push(uids_at_checkpoint(&probe, inbox.id, uid_validity)),
    )
    .await
    .expect("initial sync");

    assert_eq!(report.inserted, 7);
    assert_eq!(
        batches.first().expect("a first batch"),
        &BTreeSet::from([7, 6, 5]),
        "the newest three still land first"
    );
    assert_eq!(
        known_uids(&connection, inbox.id, uid_validity).await,
        (1..=7).collect::<BTreeSet<u32>>(),
        "every UID exactly once"
    );
    assert_eq!(
        backend.peak_in_flight(),
        1,
        "read-ahead must not put two fetches on the wire at once"
    );
}

#[tokio::test]
async fn a_sparse_uid_space_costs_one_fetch_not_one_per_two_hundred_uids() {
    let mailbox = MockMailbox::new(INBOX)
        .starting_uid(50_000)
        .message(MockMessage::new(
            "From: Ada Lovelace <ada@example.com>\r\n\
             Subject: The only survivor\r\n\r\nBody.\r\n"
                .as_bytes()
                .to_vec(),
        ));
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("sync");

    let fetches = backend.header_fetches().len();
    assert!(
        fetches <= 2,
        "a folder holding one message cost {fetches} header fetches: the pass \
         is walking the UID space (50,000 UIDs / 200 = 250 chunks) rather than \
         asking the server which UIDs exist"
    );

    let stored = known_uids(&connection, inbox.id, postio_model::Generation::new(1)).await;
    assert_eq!(
        stored.iter().copied().collect::<Vec<u32>>(),
        vec![50_000],
        "the surviving message is synced"
    );
}

#[tokio::test]
async fn a_server_that_refuses_to_list_uids_still_syncs() {
    let backend = server_with_messages(5).await;
    backend.refuse_uid_listing();

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    let report = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("a refused UID listing must not fail the pass");

    assert_eq!(report.inserted, 5, "every message still synced");
    let stored = known_uids(&connection, inbox.id, postio_model::Generation::new(1)).await;
    assert_eq!(
        stored.iter().copied().collect::<Vec<u32>>(),
        vec![1, 2, 3, 4, 5],
        "the fallback walk found the same mail"
    );
}

#[tokio::test]
async fn a_uid_listing_short_of_exists_is_not_believed() {
    let backend = server_with_messages(5).await;
    backend.truncate_uid_listing(1);

    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let (_account, inbox) = local(&connection).await;

    let report = sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("a short UID listing must not fail the pass");

    assert_eq!(
        report.inserted, 5,
        "the pass must not stop at the messages a short listing named"
    );
    let stored = known_uids(&connection, inbox.id, postio_model::Generation::new(1)).await;
    assert_eq!(
        stored.iter().copied().collect::<Vec<u32>>(),
        vec![1, 2, 3, 4, 5],
        "every message the mailbox holds is stored, not just the listed one"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_reply_with_no_references_joins_its_conversation_by_the_end_of_a_first_sync() {
    let mailbox = MockMailbox::new(INBOX)
        .message(MockMessage::new(
            b"From: Ada Lovelace <ada@example.com>\r\n\
              Message-ID: <budget@example.com>\r\n\
              Date: Mon, 1 Jun 2026 09:00:00 +0000\r\n\
              Subject: Budget\r\n\r\nThe figures.\r\n"
                .to_vec(),
        ))
        .message(MockMessage::new(
            b"From: Tove Jansson <tove@example.com>\r\n\
              Message-ID: <reply@example.com>\r\n\
              Date: Mon, 1 Jun 2026 10:00:00 +0000\r\n\
              Subject: Re: Budget\r\n\r\nLooks right.\r\n"
                .to_vec(),
        ));
    let backend = MockBackend::builder().mailbox(mailbox).build();
    backend.connect().await.expect("connect");
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (_account, inbox) = local(&connection).await;

    sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        1,
        &CancelToken::new(),
        |_| {},
    )
    .await
    .expect("the first sync");

    let messages = MessageRepository::new(&connection);
    let generation = SyncStateRepository::new(&connection)
        .get(inbox.id)
        .await
        .expect("state")
        .and_then(|state| state.generation)
        .expect("a generation");
    let thread_of = async |uid: u32| {
        messages
            .by_uid(inbox.id, generation, Uid::new(uid))
            .await
            .expect("look up")
            .expect("stored")
            .thread_id
    };
    let (original, reply) = (thread_of(1).await, thread_of(2).await);
    assert!(original.is_some() && reply.is_some(), "both are threaded");
    assert_eq!(
        original, reply,
        "`Re: Budget` is still a conversation of its own after the first sync"
    );
}
