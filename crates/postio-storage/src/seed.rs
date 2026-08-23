//! Seeding a store with realistic mail, for screenshots, UI tests and benches.
//!
//! `examples/shot.rs` in `postio-gtk` has its own hard-coded demo content today,
//! which means nothing else — a GTK test, a bench — can render the same
//! mailbox. This module is the one place that builds one, on top of the
//! ordinary repositories: an account, a folder tree, and messages filed and
//! threaded exactly as sync would file them.
//!
//! # Two variants
//!
//! [`seed_small`] draws on the `.eml` corpus ([`postio_model::test_corpus`]) via
//! [`postio_model::test_corpus::Fixture::parse`] — real, varied mail, a
//! "handful" of messages, right for a screenshot or a UI test.
//!
//! [`seed_large`] does not: at 100k+ messages, parsing the same three dozen
//! fixtures over and over would mostly measure the parser, and holding them in
//! memory first would fight the very budget CLAUDE.md sets. It builds each
//! [`Message`] directly from a small deterministic template and inserts it
//! immediately, in batches, so peak memory is one batch, never the mailbox.
//!
//! # Determinism
//!
//! Both variants take a `seed`: the same seed reproduces the same store, byte
//! for byte, so a screenshot diff or a benchmark is comparable run to run. That
//! is also why generated timestamps are measured back from a fixed [`anchor`]
//! rather than [`Utc::now`] — anchoring to the wall clock would make every run
//! a different store regardless of `seed`.
//!
//! # What is not seeded
//!
//! Message bodies and headers live in the blob store
//! ([`crate::blob::BlobStore`]), not in SQLite, and this module writes only the
//! database: [`Message::sync::body_state`](postio_model::LocalSyncState) is set
//! to [`BodyState::NotFetched`] to say so honestly, the same state a real
//! account is in before its first body backfill. Everything the message list,
//! the thread list and the sidebar's counts need — subject, preview, sender,
//! flags, dates, attachment metadata — is fully populated.
//!
//! # Availability
//!
//! Behind the `test-support` feature, alongside the rest of [`test_support`].
//!
//! [`test_support`]: crate::test_support

use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_model::{
    Account, Attachment, BodyState, EmailAddress, Flag, FlagSet, Mailbox, MailboxRole, Message,
    RfcMessageId, ids::MessageId, test_corpus,
};
use rusqlite::Connection;

use crate::db::Database;
use crate::repository::{MailboxRepository, MessageRepository, Scope, ThreadingRepository};
use crate::test_support;

/// What one seed call produced.
#[derive(Debug, Clone)]
pub struct SeedReport {
    /// The account every mailbox and message belongs to.
    pub account: Account,
    /// The folders created, with their final (recounted) [`MailboxCounts`].
    ///
    /// [`MailboxCounts`]: postio_model::MailboxCounts
    pub mailboxes: Vec<Mailbox>,
    /// How many messages were inserted.
    pub message_count: usize,
}

impl SeedReport {
    /// The folder with this role, if the seed created one.
    pub fn mailbox(&self, role: MailboxRole) -> Option<&Mailbox> {
        self.mailboxes.iter().find(|mailbox| mailbox.role == role)
    }
}

/// The folders every seed creates, by path — [`Mailbox::new`] derives each
/// one's [`MailboxRole`] from this name.
const FOLDERS: &[&str] = &["INBOX", "Archive", "Sent", "Drafts", "Trash", "Junk"];

/// How often each of [`FOLDERS`] is picked, in the same order.
///
/// Weighted toward `INBOX`, the way a real account's mail is: most of it
/// arrives and stays there, with everything else a smaller slice.
const FOLDER_WEIGHTS: &[u32] = &[60, 15, 10, 8, 4, 3];

/// How many days of spread [`seed_small`] gives its messages.
const SMALL_SPREAD_DAYS: i64 = 45;

/// How many days of spread [`seed_large`] gives its messages.
///
/// Wider than the small variant: a paging benchmark wants a cursor that walks
/// a realistic number of distinct days, not six weeks compressed into 100k
/// rows a millisecond apart.
const LARGE_SPREAD_DAYS: i64 = 730;

/// How many messages one write transaction holds, for [`seed_large`].
///
/// Bounds how much of the insert is undone if one row in the batch fails, and
/// keeps SQLite's `fsync`-per-commit cost from dominating a 100k-message seed
/// the way one commit per row would.
const BATCH_SIZE: usize = 1_000;

/// The fixed point in time seeded messages are measured back from.
///
/// Not [`Utc::now`]: a seed is supposed to be reproducible, and measuring from
/// the wall clock would make every run's timestamps different regardless of
/// `seed`.
fn anchor() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 6, 1, 9, 0, 0)
        .single()
        .expect("a fixed, valid calendar date")
}

/// Seeds `database` with the `.eml` corpus: a realistic, varied "handful" of
/// messages, right for a screenshot or a UI test.
///
/// # Panics
///
/// If a write fails — the caller has a broken store, which is a test failure
/// worth panicking on rather than threading a `Result` through every call site
/// that wants one.
pub fn seed_small(database: &Database, seed: u64) -> SeedReport {
    let connection = database.connection().expect("a checked-out connection");
    let account = test_support::account(&connection);
    let folders = create_folders(&connection, &account);
    let mut rng = Rng::new(seed);

    let mut message_count = 0;
    for fixture in test_corpus::all() {
        let mailbox = weighted_mailbox(&folders, &mut rng);
        let mut message = fixture.parse();
        message.account_id = account.id;
        message.mailbox_id = mailbox.id;
        message.received_at = recency(&mut rng, SMALL_SPREAD_DAYS);
        message.date = Some(message.received_at);
        message.flags = assign_flags(&mut rng, mailbox.role);
        message.sync.body_state = BodyState::NotFetched;

        file_message(&connection, account.id, message);
        message_count += 1;
    }

    SeedReport {
        mailboxes: recount_folders(&connection, &account),
        account,
        message_count,
    }
}

/// Seeds `database` with `message_count` synthetic messages, for the paging
/// and search benchmarks — 100k+ is the range those are meant to exercise.
///
/// Every message is built and inserted one at a time, in batches of
/// [`BATCH_SIZE`]; nothing holds more than one batch's worth in memory at once,
/// however large `message_count` is.
///
/// Unlike [`seed_small`], these messages are not threaded: none of them
/// reference each other, so every reply chain [`ThreadingRepository`] would
/// resolve is one message long, and the cost of running it 100k times would
/// buy nothing a benchmark cares about.
///
/// # Panics
///
/// If a write fails.
pub fn seed_large(database: &Database, seed: u64, message_count: usize) -> SeedReport {
    let connection = database.connection().expect("a checked-out connection");
    let account = test_support::account(&connection);
    let folders = create_folders(&connection, &account);
    let mut rng = Rng::new(seed);

    let mut inserted = 0;
    while inserted < message_count {
        let end = (inserted + BATCH_SIZE).min(message_count);
        let scope = Scope::open(&connection).expect("open a seed batch");
        for n in inserted..end {
            let mailbox = weighted_mailbox(&folders, &mut rng);
            let mut message = synthetic_message(n, &account, mailbox, &mut rng);
            MessageRepository::new(&scope)
                .create(&mut message)
                .expect("insert a synthetic message");
        }
        scope.commit().expect("commit a seed batch");
        inserted = end;
    }

    SeedReport {
        mailboxes: recount_folders(&connection, &account),
        account,
        message_count: inserted,
    }
}

fn create_folders(connection: &Connection, account: &Account) -> Vec<Mailbox> {
    FOLDERS
        .iter()
        .map(|path| test_support::mailbox(connection, account, path))
        .collect()
}

/// Recomputes and reloads every folder's cached counts.
///
/// [`MailboxRepository::create`] leaves a folder at zero, and nothing else
/// updates it as messages are inserted directly — a real sync updates counts
/// as part of filing each message, but a seed writes far more of them at once,
/// so it is cheaper to insert first and total the mailbox once at the end.
fn recount_folders(connection: &Connection, account: &Account) -> Vec<Mailbox> {
    let repository = MailboxRepository::new(connection);
    repository
        .recount_account(account.id)
        .expect("recount seeded mailboxes");
    repository
        .list_for_account(account.id)
        .expect("reload seeded mailboxes")
}

/// Inserts `message` and files it into a thread.
fn file_message(
    connection: &Connection,
    account_id: postio_model::AccountId,
    mut message: Message,
) {
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("insert a seeded message");
    ThreadingRepository::new(connection, account_id)
        .thread(&message)
        .expect("thread a seeded message");
}

/// Picks one of `folders`, weighted by [`FOLDER_WEIGHTS`].
fn weighted_mailbox<'a>(folders: &'a [Mailbox], rng: &mut Rng) -> &'a Mailbox {
    let total: u32 = FOLDER_WEIGHTS.iter().sum();
    let mut pick = rng.below(total);
    for (mailbox, weight) in folders.iter().zip(FOLDER_WEIGHTS) {
        if pick < *weight {
            return mailbox;
        }
        pick -= weight;
    }
    folders
        .last()
        .expect("create_folders always creates at least one folder")
}

/// A moment `spread_days` or fewer before [`anchor`].
fn recency(rng: &mut Rng, spread_days: i64) -> DateTime<Utc> {
    let minutes = spread_days.saturating_mul(24 * 60);
    let back = i64::from(rng.below(minutes.min(u32::MAX as i64) as u32));
    anchor() - Duration::minutes(back)
}

/// A plausible flag set for a message filed in `role`.
fn assign_flags(rng: &mut Rng, role: MailboxRole) -> FlagSet {
    let mut flags = FlagSet::new();
    match role {
        // Mail the account sent is always seen and, most of the time, a reply.
        MailboxRole::Sent => {
            flags.insert(Flag::Seen);
            if rng.chance(60) {
                flags.insert(Flag::Answered);
            }
        }
        MailboxRole::Drafts => {
            flags.insert(Flag::Draft);
        }
        _ => {
            if rng.chance(78) {
                flags.insert(Flag::Seen);
            }
            if rng.chance(15) {
                flags.insert(Flag::Answered);
            }
        }
    }
    if rng.chance(12) {
        flags.insert(Flag::Flagged);
    }
    flags
}

/// Topics [`seed_large`] draws subjects from.
const TOPICS: &[&str] = &[
    "Project status",
    "Meeting notes",
    "Invoice",
    "Weekly digest",
    "Question about the schedule",
    "Follow up",
    "Draft plan",
    "Build update",
    "Onboarding checklist",
    "Release notes",
];

/// Senders [`seed_large`] draws `From` addresses from.
const SENDERS: &[&str] = &[
    "Ada Lovelace",
    "Grace Hopper",
    "Alan Turing",
    "Katherine Johnson",
    "Margaret Hamilton",
    "Radia Perlman",
];

/// Builds one message directly, with no MIME parsing involved.
fn synthetic_message(n: usize, account: &Account, mailbox: &Mailbox, rng: &mut Rng) -> Message {
    let received_at = recency(rng, LARGE_SPREAD_DAYS);
    let mut message = Message::new(account.id, mailbox.id, received_at);
    message.date = Some(received_at);

    let sender = rng.below(SENDERS.len() as u32) as usize;
    let topic = TOPICS[rng.below(TOPICS.len() as u32) as usize];
    message.from = vec![EmailAddress::new(
        Some(SENDERS[sender]),
        format!("sender{sender}@example.com"),
    )];
    message.to = vec![account.address.clone()];
    message.subject = Some(format!("{topic} #{n}"));
    message.preview = Some(format!(
        "Generated message {n} of the large seed, on the topic of {topic}."
    ));
    message.rfc_message_id = Some(RfcMessageId::new(format!("synthetic-{n}@example.invalid")));
    message.size = 2_048 + u64::from(rng.below(8_192));

    if rng.chance(15) {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 10_240);
        attachment.filename = Some(format!("attachment-{n}.pdf"));
        message.attachments.push(attachment);
    }

    message.flags = assign_flags(rng, mailbox.role);
    message.sync.body_state = BodyState::NotFetched;
    message
}

/// A small, deterministic PRNG, so "same seed, same store" holds without
/// pulling in the `rand` crate for a dev-only fixture generator.
///
/// [SplitMix64](https://prng.di.unimi.it/splitmix64.c): not suitable for
/// anything security-sensitive, which nothing here is.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A value in `0..bound`.
    ///
    /// # Panics
    ///
    /// If `bound` is `0`.
    fn below(&mut self, bound: u32) -> u32 {
        assert!(bound > 0, "below() needs a nonzero bound");
        (self.next_u64() % u64::from(bound)) as u32
    }

    /// `true` with probability `percent` out of 100.
    fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeding_twice_with_the_same_seed_gives_the_same_store() {
        let first = seed_small(&test_support::memory(), 7);
        let second = seed_small(&test_support::memory(), 7);

        assert_eq!(first.message_count, second.message_count);
        for role in [MailboxRole::Inbox, MailboxRole::Sent, MailboxRole::Archive] {
            assert_eq!(
                first.mailbox(role).map(|m| m.counts),
                second.mailbox(role).map(|m| m.counts),
                "{role:?} disagrees between two seeds with the same key"
            );
        }
    }

    #[test]
    fn a_different_seed_gives_a_different_distribution() {
        let a = seed_small(&test_support::memory(), 1);
        let b = seed_small(&test_support::memory(), 2);

        assert_eq!(a.message_count, b.message_count, "same corpus, either way");
        assert_ne!(
            a.mailbox(MailboxRole::Inbox).unwrap().counts,
            b.mailbox(MailboxRole::Inbox).unwrap().counts,
            "different seeds should not land on exactly the same split"
        );
    }

    #[test]
    fn every_folder_exists_and_the_cached_counts_are_not_lies() {
        let database = test_support::memory();
        let report = seed_small(&database, 3);

        assert_eq!(report.mailboxes.len(), FOLDERS.len());
        let total: u32 = report.mailboxes.iter().map(|m| m.counts.total).sum();
        assert_eq!(total as usize, report.message_count);

        let connection = database.connection().unwrap();
        for mailbox in &report.mailboxes {
            let actual = MessageRepository::new(&connection)
                .count(&crate::repository::ListQuery::mailbox(mailbox.id))
                .unwrap();
            assert_eq!(
                actual, mailbox.counts.total,
                "{}'s cached total does not match what is actually there",
                mailbox.path
            );
        }
    }

    #[test]
    fn corpus_replies_land_in_the_same_thread_as_their_root() {
        let database = test_support::memory();
        let report = seed_small(&database, 11);
        let connection = database.connection().unwrap();

        let root = test_corpus::load("list-thread-01-root").parse();
        let threading = ThreadingRepository::new(&connection, report.account.id);
        let thread_id = threading
            .thread_of(root.rfc_message_id.as_ref().unwrap())
            .expect("looking up the root's thread must not fail")
            .expect("the root fixture was seeded and threaded");

        let reply = test_corpus::load("list-thread-02-reply").parse();
        assert_eq!(
            threading
                .thread_of(reply.rfc_message_id.as_ref().unwrap())
                .expect("looking up the reply's thread must not fail"),
            Some(thread_id),
            "a reply fixture must land in its root's thread"
        );
    }

    #[test]
    fn no_message_pretends_to_have_a_body_that_was_never_written() {
        let database = test_support::memory();
        let report = seed_small(&database, 4);
        let connection = database.connection().unwrap();

        for mailbox in &report.mailboxes {
            let rows = MessageRepository::new(&connection)
                .page(&crate::repository::ListQuery::mailbox(mailbox.id).limit(u32::MAX))
                .unwrap();
            for row in rows {
                let message = MessageRepository::new(&connection)
                    .get(row.id)
                    .unwrap()
                    .expect("the row just listed");
                assert_eq!(message.sync.body_state, BodyState::NotFetched);
                assert!(message.raw_blob_id.is_none());
            }
        }
    }

    #[test]
    fn the_large_variant_inserts_exactly_as_many_messages_as_asked() {
        let database = test_support::memory();
        let report = seed_large(&database, 5, 250);

        assert_eq!(report.message_count, 250);
        let total: u32 = report.mailboxes.iter().map(|m| m.counts.total).sum();
        assert_eq!(total, 250);
    }

    #[test]
    fn the_large_variant_batches_across_more_than_one_transaction() {
        // A message count that spans several BATCH_SIZE-sized transactions,
        // to prove batching does not drop or duplicate rows at the seam.
        let database = test_support::memory();
        let report = seed_large(&database, 9, BATCH_SIZE * 2 + 137);

        assert_eq!(report.message_count, BATCH_SIZE * 2 + 137);
        let total: u32 = report.mailboxes.iter().map(|m| m.counts.total).sum();
        assert_eq!(total as usize, report.message_count);
    }

    #[test]
    fn rng_below_stays_in_bounds_and_is_reproducible_from_its_seed() {
        let mut a = Rng::new(42);
        let mut b = Rng::new(42);
        for _ in 0..1_000 {
            let (x, y) = (a.below(17), b.below(17));
            assert_eq!(x, y);
            assert!(x < 17);
        }
    }
}
