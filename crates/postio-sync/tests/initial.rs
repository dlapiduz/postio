//! Initial sync: newest UID first, resumable, threading as it goes.
//!
//! No network and no server: `MockBackend` is the in-memory mail store the
//! whole sync engine is developed against (see
//! `crates/postio-imap/src/backend/mod.rs`).

use postio_imap::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_imap::cancel::CancelToken;
use postio_model::{AccountId, Mailbox, MailboxId, Uid};
use postio_storage::repository::{ContactRepository, MessageRepository, SyncStateRepository};
use postio_storage::test_support;
use postio_sync::{Progress, sync_mailbox, sync_mailbox_with_batch_size};
use rusqlite::Connection;
use std::collections::BTreeSet;

const INBOX: &str = "INBOX";

/// A server with `count` messages in `INBOX`, oldest seeded first so UIDs run
/// `1..=count` in seeding order.
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

/// An account with an empty local `INBOX` — nothing synced yet.
fn local(connection: &Connection) -> (AccountId, Mailbox) {
    let account = test_support::account(connection);
    let inbox = test_support::mailbox(connection, &account, INBOX);
    (account.id, inbox)
}

fn known_uids(
    connection: &Connection,
    mailbox_id: MailboxId,
    uid_validity: postio_model::UidValidity,
) -> BTreeSet<u32> {
    MessageRepository::new(connection)
        .uids_in(mailbox_id, uid_validity)
        .expect("uids_in")
        .into_iter()
        .map(Uid::get)
        .collect()
}

#[tokio::test]
async fn the_newest_batch_lands_before_the_oldest_one() {
    let backend = server_with_messages(5).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);

    let mut batches: Vec<BTreeSet<u32>> = Vec::new();
    let uid_validity = postio_model::UidValidity::new(1);
    sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        2,
        &CancelToken::new(),
        |_progress: Progress| {
            batches.push(known_uids(&connection, inbox.id, uid_validity));
        },
    )
    .await
    .expect("initial sync");

    // Batch size 2 over UIDs 1..=5, walked newest first: {5,4}, then {5,4,3,2},
    // then everything. The mailbox's whole state at the first checkpoint is
    // exactly the two newest messages — nothing older has been touched yet.
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
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);

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
                // Simulate the process dying right after the first batch
                // commits: the *next* cancellation check stops the pass.
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
        known_uids(&connection, inbox.id, postio_model::UidValidity::new(1)),
        BTreeSet::from([4, 5]),
        "only the first batch should have committed"
    );
    assert!(
        !SyncStateRepository::new(&connection)
            .require(inbox.id)
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

    // Only the three UIDs missing after the first pass were fetched — proof
    // the resumed pass never asked the server for 4 or 5 again.
    assert_eq!(resumed.inserted, 3);
    assert_eq!(resumed.updated, 0);
    assert_eq!(
        known_uids(&connection, inbox.id, postio_model::UidValidity::new(1)),
        BTreeSet::from([1, 2, 3, 4, 5])
    );
    assert!(
        SyncStateRepository::new(&connection)
            .require(inbox.id)
            .expect("sync state")
            .has_synced()
    );
}

#[tokio::test]
async fn a_reply_arriving_before_its_parent_still_finds_its_thread() {
    let backend = MockBackend::builder()
        .mailbox(
            MockMailbox::new(INBOX)
                // UID 1: the parent, seeded (and therefore fetched) last.
                .message(MockMessage::new(
                    b"From: Ada Lovelace <ada@example.com>\r\n\
                      Subject: Analytical engine\r\n\
                      Message-ID: <parent@example.com>\r\n\r\nNotes.\r\n"
                        .to_vec(),
                ))
                // UID 2: the reply, higher UID, fetched first in newest-first
                // order — before the parent it answers has even been asked
                // for.
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

    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);

    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("initial sync");

    let messages = MessageRepository::new(&connection);
    let parent = messages
        .by_uid(inbox.id, postio_model::UidValidity::new(1), Uid::new(1))
        .expect("look up parent")
        .expect("parent stored");
    let reply = messages
        .by_uid(inbox.id, postio_model::UidValidity::new(1), Uid::new(2))
        .expect("look up reply")
        .expect("reply stored");

    assert!(parent.thread_id.is_some());
    assert_eq!(
        parent.thread_id, reply.thread_id,
        "the reply's thread must have claimed the parent once it arrived, \
         even though the reply was threaded first"
    );
}

/// postio-66j: `ContactRepository::record_message` existed, was tested, and
/// had no caller, so @ and recipient completion always listed nobody no
/// matter how much mail the account held. This is the seam that broke —
/// before the fix this assertion fails with an empty contact list, because
/// nothing in `sync_mailbox`'s call chain ever wrote the `contacts` table.
#[tokio::test]
async fn a_full_sync_records_every_correspondent_as_a_contact() {
    let backend = server_with_messages(3).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account_id, inbox) = local(&connection);

    sync_mailbox(&connection, &backend, &inbox, &CancelToken::new(), |_| {})
        .await
        .expect("initial sync");

    let contacts = ContactRepository::new(&connection)
        .list(Some(account_id))
        .expect("list contacts");
    let ada = contacts
        .iter()
        .find(|contact| contact.address.normalized() == "ada@example.com")
        .expect("the sender of all three messages must be a recorded contact");
    assert_eq!(
        ada.times_seen, 3,
        "one sighting per message, not per address occurrence"
    );
}

/// `postio-qhz.9`: what a progress report is a fraction *of*.
///
/// The denominator used to be `UIDNEXT - 1`, which is the UID range the pass
/// enumerates and not the number of messages in the folder. Every message ever
/// expunged is still counted in it, so a long-lived INBOX reported
/// `done=61 total=63022` for ninety-two messages and the status line read
/// `syncing 0% · imap` for the whole pass. The bead this came from asked for
/// the opposite: the user should be able to tell four hundred from forty
/// thousand.
///
/// A mailbox seeded from UID 1 cannot catch this — the ceiling and the count
/// are the same number — which is why `MockMailbox::starting_uid` exists.
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

    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account_id, inbox) = local(&connection);

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

#[tokio::test]
async fn the_next_batch_is_asked_for_before_this_one_is_committed() {
    // #77: within one mailbox the pass used to be strictly request/response —
    // ask, wait, commit, ask again — so the connection sat idle for the whole
    // local write. The next FETCH now goes out *before* the write, which puts
    // the server to work on batch n+1 while SQLite is still taking batch n.
    //
    // Asserted on the mock's causal fetch log rather than a stopwatch (#125):
    // "how many fetches had the server served by the time this batch was
    // committed?" A sequential pass can only ever answer "this one".
    let backend = server_with_messages(6).await;
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);

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

    // Six messages in batches of two: three batches. When the first is
    // committed the second has already been asked for; when the second is
    // committed so has the third; the third has nothing to run ahead of.
    assert_eq!(
        served_at_commit,
        vec![2, 3, 3],
        "each batch should be requested while its predecessor is being written"
    );
}

#[tokio::test]
async fn running_ahead_never_reorders_or_loses_a_batch() {
    // The pipelining must not disturb what #32's constraints pin: newest
    // first, every message exactly once, and a resumable pass. Latency here
    // is what makes the read-ahead genuinely outstanding during the write
    // rather than answered inside the priming poll.
    let backend = server_with_messages(7).await;
    backend.set_latency(std::time::Duration::from_millis(5));
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (_account, inbox) = local(&connection);

    let uid_validity = postio_model::UidValidity::new(1);
    let mut batches: Vec<BTreeSet<u32>> = Vec::new();
    let report = sync_mailbox_with_batch_size(
        &connection,
        &backend,
        &inbox,
        3,
        &CancelToken::new(),
        |_progress: Progress| batches.push(known_uids(&connection, inbox.id, uid_validity)),
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
        known_uids(&connection, inbox.id, uid_validity),
        (1..=7).collect::<BTreeSet<u32>>(),
        "every UID exactly once"
    );
    // One outstanding fetch at a time: a pass holds one pooled connection and
    // read-ahead must not want a second.
    assert_eq!(
        backend.peak_in_flight(),
        1,
        "read-ahead must not put two fetches on the wire at once"
    );
}
