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
use rusqlite::{Connection, params};

use crate::db::Database;
use crate::repository::{
    ContactRepository, MailboxRepository, MessageRepository, Scope, StoredBody, ThreadingRepository,
};
use crate::test_support;

/// What one seed call produced.
#[derive(Debug, Clone)]
pub struct SeedReport {
    /// The account every mailbox and message belongs to.
    pub account: Account,
    /// The folders created, with the counts the inserts produced.
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
    seed_small_into(database, false, seed)
}

/// [`seed_small`], plus the corpus' own bodies written into `blobs`.
///
/// The difference matters to anything that renders a message rather than
/// listing one. `seed_small` writes only the database and says so honestly
/// with [`BodyState::NotFetched`], which is the state a real account is in
/// before its first backfill — so a reader fed from it draws the "still
/// downloading" plate, never mail. That is right for a test about the plate
/// and wrong for a screenshot of the reading pane, which was reduced to
/// handing the reader a body of its own invention and so could not fail when
/// the path from the store was broken (#596).
///
/// The bodies are the fixtures', decoded by the same `mime::parse` the sync
/// path uses, so what is rendered is what the corpus holds.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub fn seed_small_with_bodies(database: &Database, seed: u64) -> SeedReport {
    seed_small_into(database, true, seed)
}

fn seed_small_into(database: &Database, with_bodies: bool, seed: u64) -> SeedReport {
    let connection = database.connection().expect("a checked-out connection");
    let account = test_support::account(&connection);
    let folders = create_folders(&connection, &account);
    let mut rng = Rng::new(seed);

    let mut message_count = 0;
    for fixture in test_corpus::all() {
        let mailbox = weighted_mailbox(&folders, &mut rng);
        let received_at = recency(&mut rng, SMALL_SPREAD_DAYS);
        let parsed = postio_model::mime::parse(fixture.bytes());
        let body = parsed.body.clone();
        let mut message = parsed.into_message(account.id, mailbox.id, received_at);
        message.account_id = account.id;
        message.mailbox_id = mailbox.id;
        message.received_at = received_at;
        message.date = Some(message.received_at);
        message.flags = assign_flags(&mut rng, mailbox.role);
        message.sync.body_state = BodyState::NotFetched;

        let id = file_message(&connection, account.id, message);
        if with_bodies {
            write_body(&connection, id, &body);
        }
        message_count += 1;
    }

    SeedReport {
        mailboxes: load_folders(&connection, &account),
        account,
        message_count,
    }
}

/// Add a second account, with its own folder tree and a share of the corpus.
///
/// [`seed_small`] seeds one account, which is the shape almost everything
/// wants. The sidebar's per-account sections and the unified scope cannot be
/// looked at or tested with one, and a fixture that fakes the second account
/// at the widget instead cannot fail when the wiring is broken (#185).
///
/// The messages are the corpus's again, filed into this account's folders, so
/// a unified list has two accounts' mail in it and the rows have somewhere to
/// get an account from.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub fn seed_extra_account(
    database: &Database,
    display: &str,
    address: &str,
    seed: u64,
) -> SeedReport {
    let connection = database.connection().expect("a checked-out connection");
    let mut account = Account::new(display, EmailAddress::new(Some(display), address));
    account.incoming.host = "imap.example.net".to_owned();
    account.outgoing.host = "smtp.example.net".to_owned();
    crate::repository::AccountRepository::new(&connection)
        .create(&mut account)
        .expect("create a seeded account");

    let folders = create_folders(&connection, &account);
    let mut rng = Rng::new(seed);
    let mut message_count = 0;
    for fixture in test_corpus::all() {
        let mailbox = weighted_mailbox(&folders, &mut rng);
        let received_at = recency(&mut rng, SMALL_SPREAD_DAYS);
        let mut message = fixture.parse();
        message.account_id = account.id;
        message.mailbox_id = mailbox.id;
        message.received_at = received_at;
        message.date = Some(received_at);
        message.flags = assign_flags(&mut rng, mailbox.role);
        message.sync.body_state = BodyState::NotFetched;
        file_message(&connection, account.id, message);
        message_count += 1;
    }

    SeedReport {
        mailboxes: load_folders(&connection, &account),
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
            record_correspondents(&scope, &message);
        }
        scope.commit().expect("commit a seed batch");
        inserted = end;
    }

    SeedReport {
        mailboxes: load_folders(&connection, &account),
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

/// Reloads every folder, with whatever counts the inserts left behind.
///
/// It used to call [`MailboxRepository::recount_account`] here, and that one
/// line hid a shipped bug for the life of the project. A seeded store came
/// out with correct cached counts; a real one did not, because nothing
/// maintained them — so the message list drew rows from every fixture and
/// nothing from a live account, and no test could tell.
///
/// A fixture must not supply by hand what the application is supposed to
/// produce. Counts now come from migration 0003's triggers, which is the same
/// mechanism a real sync goes through, so a seeded store is only listable if
/// the production path works. If these counts are ever wrong again, that is a
/// bug in the triggers and it should be found here rather than papered over.
/// See `postio-bl2`.
fn load_folders(connection: &Connection, account: &Account) -> Vec<Mailbox> {
    MailboxRepository::new(connection)
        .list_for_account(account.id)
        .expect("reload seeded mailboxes")
}

/// Groups an already-seeded account's messages into conversations of
/// `per_thread`, for benchmarking the threaded list. Answers how many threads.
///
/// [`seed_large`] deliberately leaves its messages unthreaded — it exists for
/// the *message* window, where threading would only add noise. The thread
/// list needs the opposite, and needs it at a size worth measuring, so this
/// threads a seeded store after the fact.
///
/// **Not how threading works.** Real threading is JWZ over `References` and
/// `In-Reply-To` (`ThreadingRepository`), one message at a time, and running
/// it over a hundred thousand synthetic messages would measure the seeder
/// rather than the query. This assigns membership in bulk and then computes
/// the aggregates in one statement, which produces the same *shape* of data —
/// which is all a read benchmark is about.
///
/// # Panics
///
/// If the store cannot be written.
pub fn thread_seeded_messages(
    database: &Database,
    account: postio_model::AccountId,
    per_thread: usize,
) -> u32 {
    assert!(per_thread > 0, "a conversation holds at least one message");
    let connection = database.connection().expect("a checked-out connection");

    let rows: Vec<(i64, Option<String>)> = {
        let mut statement = connection
            .prepare(
                "SELECT id, subject FROM messages WHERE account_id = ?1
                  ORDER BY received_at DESC, id DESC",
            )
            .expect("prepare the seeded message list");
        let rows = statement
            .query_map([account.get()], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
            })
            .expect("read the seeded message list");
        rows.collect::<rusqlite::Result<_>>()
            .expect("collect the seeded message list")
    };

    let scope = Scope::open(&connection).expect("open a threading batch");
    let mut threads = 0;
    for chunk in rows.chunks(per_thread) {
        // A real thread's subject is one of its own messages' (`recompute_in`
        // reads the oldest member's), never a constant -- and a benchmark
        // that gave every seeded thread the identical literal subject once
        // sent `unified_page`'s subject-coalescing query chasing all 13,000
        // of them as candidates for every page row, which is what #619's
        // budget miss actually was, not a query-plan problem: instrumenting
        // confirmed every one of the top 100 raw threads shared this one
        // literal subject, and fixing only that (nothing in `unified_page`
        // itself) took `cargo bench`'s own measurement from 18.6-19.5ms to
        // 1.7-1.9ms. Any member's subject keeps every seeded thread's
        // subject as distinct as its messages' already are, which is "the
        // same shape of data" this function promises rather than a
        // pathological one no real mailbox produces.
        let subject = chunk
            .first()
            .and_then(|(_, subject)| subject.as_deref())
            .unwrap_or("seeded conversation");
        scope
            .execute(
                "INSERT INTO threads (account_id, subject, message_count, unread_count,
                                      has_attachments, is_flagged, first_at, last_at)
                 VALUES (?1, ?2, 0, 0, 0, 0, 0, 0)",
                params![account.get(), subject],
            )
            .expect("insert a seeded thread");
        let thread = scope.last_insert_rowid();
        for (id, _) in chunk {
            scope
                .execute(
                    "UPDATE messages SET thread_id = ?1 WHERE id = ?2",
                    [thread, *id],
                )
                .expect("file a seeded message into its thread");
        }
        threads += 1;
    }
    // The aggregates, in one statement rather than per thread.
    scope
        .execute(
            "UPDATE threads SET
                 message_count = (SELECT count(*) FROM messages m
                                   WHERE m.thread_id = threads.id AND m.deleted_locally = 0),
                 unread_count  = (SELECT count(*) FROM messages m
                                   WHERE m.thread_id = threads.id AND m.deleted_locally = 0
                                     AND m.seen = 0),
                 first_at = coalesce((SELECT min(received_at) FROM messages m
                                       WHERE m.thread_id = threads.id), 0),
                 last_at  = coalesce((SELECT max(received_at) FROM messages m
                                       WHERE m.thread_id = threads.id), 0)
               WHERE account_id = ?1",
            [account.get()],
        )
        .expect("recompute the seeded thread aggregates");
    scope.commit().expect("commit the threading batch");
    threads
}

/// Inserts `message`, files it into a thread, and remembers who wrote it.
/// Write `body` into `blobs` and point `id` at it.
///
/// Compressed into the row by the repository, the same path real mail takes
/// (ADR 0020).
fn write_body(connection: &Connection, id: MessageId, body: &postio_model::MessageBody) {
    if body.text.is_none() && body.html.is_none() {
        // Nothing to store. The row keeps `NotFetched`, which is true: this
        // fixture has no body to have fetched.
        return;
    }
    MessageRepository::new(connection)
        .set_body(
            id,
            &StoredBody {
                text: body.text.clone(),
                html: body.html.clone(),
                headers: None,
            },
            BodyState::Full,
        )
        .expect("store a seeded body");
}

fn file_message(
    connection: &Connection,
    account_id: postio_model::AccountId,
    mut message: Message,
) -> MessageId {
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("insert a seeded message");
    ThreadingRepository::new(connection, account_id)
        .thread(&message)
        .expect("thread a seeded message");
    record_correspondents(connection, &message);
    message.id
}

/// Remember everyone on `message`, the way a sync pass would.
///
/// Contacts are accumulated by `postio-sync` as mail arrives, and a seeded
/// store never goes near that path — so before `postio-3ta` every screenshot,
/// demo, bench and UI test built on one had an empty `@` palette and an empty
/// recipient completion however much mail was in the store. A fixture that
/// claims to model a synced account has to model this too, or the surfaces
/// that read it cannot be told apart from the ones nothing ever wired up.
///
/// Not fatal: a sighting that will not record leaves the store's *mail*
/// perfectly good, and panicking here would turn a completion list into a
/// broken fixture.
fn record_correspondents(connection: &Connection, message: &Message) {
    if let Err(error) = ContactRepository::new(connection).record_message(message) {
        tracing::warn!(%error, "could not record a seeded message's correspondents");
    }
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
    use crate::repository::ContactRepository;

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
    fn seeding_with_bodies_writes_mail_the_reader_can_actually_read() {
        let database = test_support::memory();
        let report = seed_small_with_bodies(&database, 11);

        let connection = database.connection().expect("a connection");
        let repository = MessageRepository::new(&connection);
        let page = repository
            .page(&crate::repository::ListQuery::account(report.account.id).limit(u32::MAX))
            .expect("the seeded messages");

        let mut readable = 0;
        for row in &page {
            let Some(body) = repository.body(row.id).expect("a body record") else {
                continue;
            };
            for text in [body.text, body.html].into_iter().flatten() {
                assert!(!text.is_empty(), "a body was stored empty");
                readable += 1;
            }
        }
        // Not merely "some row has a blob id": the point of this seed is that
        // the bytes are there to be read back, because a reader fed from it
        // renders mail rather than the "still downloading" plate.
        assert!(
            readable > 0,
            "seeded {} messages and not one had a body that read back",
            report.message_count
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
    fn a_seeded_store_knows_who_has_written_to_it() {
        // `postio-3ta`. Contacts are recorded by the *sync* path, and a seeded
        // store never goes near it — so every screenshot, demo and test built
        // on one had an `@` palette and a recipient completion that were
        // empty however much mail was in the store. A fixture that models a
        // synced account has to model this too, or the surfaces that read it
        // cannot be told apart from the ones nobody wired up.
        let database = test_support::memory();
        let report = seed_small(&database, 7);
        let connection = database.connection().expect("a checked-out connection");

        let contacts = ContactRepository::new(&connection)
            .search(Some(report.account.id), "", 1_000)
            .expect("read the seeded correspondents");

        assert!(
            !contacts.is_empty(),
            "the seed filed {} messages and the store knows nobody who sent \
             one of them",
            report.message_count
        );
        // Not merely non-empty: they have to be the mail's own senders. A
        // fixture that invented correspondents would pass the line above and
        // still not resemble a synced account.
        let senders: std::collections::BTreeSet<String> = MessageRepository::new(&connection)
            .page(&crate::repository::ListQuery::account(report.account.id).limit(u32::MAX))
            .expect("read the seeded mail")
            .into_iter()
            .filter_map(|row| row.from)
            .map(|from| from.normalized())
            .collect();
        assert!(
            contacts
                .iter()
                .any(|contact| senders.contains(&contact.address.normalized())),
            "the store lists correspondents that never wrote any of the \
             seeded mail"
        );
    }

    #[test]
    fn the_large_variant_records_its_correspondents_too() {
        // Its senders come from a small fixed pool, so this is a handful of
        // rows however many messages there are — and the benches and paging
        // fixtures built on it look like an account somebody actually uses.
        let database = test_support::memory();
        let report = seed_large(&database, 9, 500);
        let connection = database.connection().expect("a checked-out connection");

        let contacts = ContactRepository::new(&connection)
            .search(Some(report.account.id), "", 1_000)
            .expect("read the seeded correspondents");

        assert!(
            !contacts.is_empty(),
            "500 synthetic messages and not one recorded sender"
        );
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
