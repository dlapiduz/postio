//! Seeding a store with realistic mail, for screenshots, UI tests and benches.
//!
//! This module is the one place that builds a demo mailbox, so a screenshot,
//! a storyboard, a GTK test and a bench all render the same one: Postio's
//! `postio_gtk::demo` builds its seeds' store halves here. It works on top
//! of the ordinary repositories: an account, a folder tree, and messages
//! filed and threaded exactly as sync would file them.
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
    RfcMessageId,
    ids::{AccountId, MessageId},
    test_corpus,
};

use crate::repository::{
    ContactRepository, MailboxRepository, MessageRepository, StoredBody, ThreadingRepository,
};
use crate::sql::{self, RowExt as _, bind};
use crate::store::{Connection, Store};
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

/// The conversation lengths of a storyboard's `thirty-threads` seed, for
/// [`seed_conversations`]: thirty-six threads, mostly single messages, some
/// exchanges, one of five (specs/008-storyboards R11).
pub const THIRTY_THREADS: &[usize] = &[
    1, 2, 1, 1, 3, 1, 1, 2, 1, 1, 1, 4, 1, 2, 1, 1, 1, 3, 1, 1, 2, 1, 1, 1, 5, 1, 1, 2, 1, 1, 1, 3,
    1, 1, 2, 1,
];

/// The one conversation of a storyboard's `long-thread` seed, for
/// [`seed_conversations`]: seven messages, the last two unread.
pub const LONG_THREAD: &[usize] = &[7];

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
pub async fn seed_small(database: &Store, seed: u64) -> SeedReport {
    seed_corpus(database, seed, false).await
}

/// [`seed_small`] with every body already downloaded: the store of an
/// account that has been open a while, where a hit from search or a reply
/// finds the message's text and not an "original still downloading" note.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub async fn seed_small_downloaded(database: &Store, seed: u64) -> SeedReport {
    seed_corpus(database, seed, true).await
}

async fn seed_corpus(database: &Store, seed: u64, downloaded: bool) -> SeedReport {
    let connection = database.connect().await.expect("a checked-out connection");
    let account = seeded_account(&connection).await;
    let folders = create_folders(&connection, &account).await;
    let mut rng = Rng::new(seed);

    let mut message_count = 0;
    for fixture in test_corpus::all() {
        let mailbox = weighted_mailbox(&folders, &mut rng);
        let received_at = recency(&mut rng, SMALL_SPREAD_DAYS);
        let parsed = postio_model::mime::parse(fixture.bytes());
        let mut message = parsed.into_message(account.id, mailbox.id, received_at);
        message.account_id = account.id;
        message.mailbox_id = mailbox.id;
        message.received_at = received_at;
        message.date = Some(message.received_at);
        message.flags = assign_flags(&mut rng, mailbox.role);
        message.sync.body_state = BodyState::NotFetched;
        let body = message.body.clone();

        let id = file_message(&connection, account.id, message).await;
        if downloaded && !body.is_empty() {
            MessageRepository::new(&connection)
                .set_body(
                    id,
                    &crate::repository::StoredBody {
                        text: body.text,
                        html: body.html,
                        headers: None,
                        headers_truncated: false,
                        encoding_problems: false,
                    },
                    BodyState::Full,
                )
                .await
                .expect("store a seeded body");
        }
        message_count += 1;
    }

    SeedReport {
        mailboxes: load_folders(&connection, &account).await,
        account,
        message_count,
    }
}

/// The seeded account: `test_support::account` plus the identity a real one
/// always has.
///
/// Onboarding gives every account it creates an identity
/// (`postio_session::onboarding`), so a seed without one describes a state the
/// application cannot produce — and anything resting on it rests on a state
/// that does not occur. The cost was visible rather than theoretical: a reply
/// driven through the real path rendered "no identity configured" in its
/// `From` row, so every picture of the composer looked broken in a way the
/// running application never is.
///
/// Here rather than in `test_support::account`, which is the minimal building
/// block tests compose their own identities onto. Several add a default of
/// their own and `idx_identities_one_default` allows exactly one, so putting
/// it there makes three suites collide over a fixture they did not ask to
/// change.
async fn seeded_account(connection: &Connection) -> Account {
    let mut account = test_support::account(connection).await;
    // Not marked default, which is the one concession to the fixtures around
    // it. `idx_identities_one_default` allows a single default per account and
    // several suites add a default of their own to a seeded account; claiming
    // it here makes three of them collide over a fixture they did not ask to
    // change. Nothing is lost by not claiming it — `Account::identity_for`
    // falls back to the first identity, so this is still the one a reply sends
    // as.
    let identity = postio_model::Identity::new(account.id, account.address.clone());
    account.identities = vec![identity];
    crate::repository::AccountRepository::new(connection)
        .update(&mut account)
        .await
        .expect("give the seeded account its identity");
    account
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
pub async fn seed_extra_account(
    database: &Store,
    display: &str,
    address: &str,
    seed: u64,
) -> SeedReport {
    let connection = database.connect().await.expect("a checked-out connection");
    let mut account = Account::new(display, EmailAddress::new(Some(display), address));
    account.incoming.host = "imap.example.net".to_owned();
    account.outgoing.host = "smtp.example.net".to_owned();
    // An identity, because onboarding gives every real account one
    // (`postio_session::onboarding`) and a seed that does not builds an account
    // the application itself cannot produce. What that costs is not
    // hypothetical: a reply driven through the real path renders "no identity
    // configured" in its `From` row, so every picture of the composer looks
    // broken in a way the running application never is.
    let mut identity =
        postio_model::Identity::new(account.id, EmailAddress::new(Some(display), address));
    identity.is_default = true;
    account.identities = vec![identity];
    crate::repository::AccountRepository::new(&connection)
        .create(&mut account)
        .await
        .expect("create a seeded account");

    let folders = create_folders(&connection, &account).await;
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
        file_message(&connection, account.id, message).await;
        message_count += 1;
    }

    SeedReport {
        mailboxes: load_folders(&connection, &account).await,
        account,
        message_count,
    }
}

/// Seeds an account whose Inbox holds exactly these conversations, newest
/// first: `lengths[0]` messages in the newest, and so on. An empty slice is an
/// account with its folders and no mail.
///
/// For a demo or a storyboard that needs a mailbox of a known shape -- thirty
/// threads to scroll, one long conversation to walk -- rather than the
/// corpus' mixed handful. Every message has a text body in the store, and
/// each conversation's later messages alternate between the correspondent and
/// the account, so a thread reads as an exchange. The newest messages of a
/// conversation are the unread ones: the last of any of three or more, the
/// last two of six or more, and the last of a shorter one when the seed says
/// so.
///
/// Deterministic in `seed`, and anchored at the fixed [`anchor`] rather than
/// the wall clock. Addresses are `example.com`.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub async fn seed_conversations(database: &Store, seed: u64, lengths: &[usize]) -> SeedReport {
    let connection = database.connect().await.expect("a checked-out connection");
    let account = seeded_account(&connection).await;
    let folders = create_folders(&connection, &account).await;
    let inbox = folders
        .iter()
        .find(|mailbox| mailbox.role == MailboxRole::Inbox)
        .expect("create_folders always creates an Inbox");
    let mut rng = Rng::new(seed);

    let mut number = 0usize;
    let mut offset = Duration::zero();
    let mut message_count = 0;
    for (index, &length) in lengths.iter().enumerate() {
        let topic = TOPICS[index % TOPICS.len()];
        let sender = index % SENDERS.len();
        let correspondent =
            EmailAddress::new(Some(SENDERS[sender]), format!("sender{sender}@example.com"));
        let tail = if length >= 6 { 2 } else { 1 };
        for position in 0..length {
            // Oldest first within the conversation, the whole run before the
            // next conversation's, so sorting by time keeps each one whole.
            let received_at =
                anchor() - offset - Duration::minutes(15 * (length - 1 - position) as i64);
            let mut message = Message::new(account.id, inbox.id, received_at);
            message.date = Some(received_at);
            let from_correspondent = position % 2 == 0;
            let (from, to) = if from_correspondent {
                (correspondent.clone(), account.address.clone())
            } else {
                (account.address.clone(), correspondent.clone())
            };
            message.from = vec![from];
            message.to = vec![to];
            message.subject = Some(if position == 0 {
                format!("{topic} ({})", index + 1)
            } else {
                format!("Re: {topic} ({})", index + 1)
            });
            let text = format!(
                "Message {} of a conversation about {topic}, from the demo seed.",
                position + 1
            );
            message.preview = Some(text.clone());
            message.rfc_message_id = Some(RfcMessageId::new(format!(
                "conversation-{number}@example.invalid"
            )));
            message.size = 1_024 + u64::from(rng.below(2_048));
            let in_tail = position + tail >= length;
            let unread = in_tail && (length >= 3 || rng.chance(35));
            let mut flags = FlagSet::new();
            if !unread {
                flags.insert(Flag::Seen);
            }
            message.flags = flags;
            message.sync.body_state = BodyState::NotFetched;

            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("insert a seeded message");
            write_body(
                &connection,
                message.id,
                &postio_model::MessageBody {
                    text: Some(text),
                    html: None,
                },
            )
            .await;
            record_correspondents(&connection, &message).await;
            number += 1;
            message_count += 1;
        }
        offset += Duration::minutes(15 * length as i64) + Duration::hours(3);
    }
    drop(connection);

    let mut runs = std::collections::VecDeque::from(lengths.to_vec());
    thread_in_runs(database, account.id, move || runs.pop_front().unwrap_or(1)).await;

    let connection = database.connect().await.expect("a checked-out connection");
    SeedReport {
        mailboxes: load_folders(&connection, &account).await,
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
/// Threaded in conversations of mixed length, the way a real folder lists
/// them -- mostly single messages, some exchanges, a few long threads -- by
/// the same bulk assignment [`thread_seeded_messages`] makes rather than by
/// running [`ThreadingRepository`] a hundred thousand times. They used to be
/// left unthreaded, and a folder of them listed plain message rows, which is
/// not what any real folder lists: every large-fixture test walked a path
/// no user takes, and landing on a conversation asked for the whole folder
/// (138 page requests for five `j` presses) with none of them able to see it.
///
/// # Panics
///
/// If a write fails.
pub async fn seed_large(database: &Store, seed: u64, message_count: usize) -> SeedReport {
    let connection = database.connect().await.expect("a checked-out connection");
    let account = seeded_account(&connection).await;
    let folders = create_folders(&connection, &account).await;
    let mut rng = Rng::new(seed);

    let mut inserted = 0;
    while inserted < message_count {
        let end = (inserted + BATCH_SIZE).min(message_count);
        // Generated before the transaction opens, written inside it. The
        // generator is pure and holds `rng` and `folders`, which an `async
        // move` closure would take from the loop that still needs them.
        let batch: Vec<_> = (inserted..end)
            .map(|n| {
                let mailbox = weighted_mailbox(&folders, &mut rng);
                synthetic_message(n, &account, mailbox, &mut rng)
            })
            .collect();

        sql::in_scope(&connection, move |scope| async move {
            for mut message in batch {
                MessageRepository::new(&scope)
                    .create(&mut message)
                    .await
                    .expect("insert a synthetic message");
                record_correspondents(&scope, &message).await;
            }
            Ok::<_, crate::Error>(())
        })
        .await
        .expect("commit a seed batch");
        inserted = end;
    }
    drop(connection);

    thread_in_runs(database, account.id, move || conversation_length(&mut rng)).await;

    let connection = database.connect().await.expect("a checked-out connection");
    SeedReport {
        mailboxes: load_folders(&connection, &account).await,
        account,
        message_count: inserted,
    }
}

/// Adds `count` messages to `report`'s inbox, received over the 30 days
/// before `now` and sent to the account, each with a plain-text body here --
/// the mail Focus's needs-action pass reads (spec 007 FR-141, SC-011).
/// Answers their ids.
///
/// [`seed_large`] anchors its mail months back and stores no body, which is
/// right for a list and wrong for this pass: it reads only recent inbox mail
/// whose body is on this machine. The bodies are shaped like a working
/// person's mail -- a greeting, an ask or an update, a signature, and often
/// quoted history -- so the pass cuts own text and reads sentences as it
/// would in a real store. About a third ask something; the rest do not.
///
/// # Panics
///
/// If a write fails.
pub async fn seed_recent_with_bodies(
    database: &Store,
    report: &SeedReport,
    count: usize,
    now: DateTime<Utc>,
    seed: u64,
) -> Vec<MessageId> {
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed made an inbox")
        .clone();
    let account = report.account.clone();
    let connection = database.connect().await.expect("a checked-out connection");
    let mut rng = Rng::new(seed);
    let mut ids = Vec::with_capacity(count);
    let mut made = 0;
    while made < count {
        let end = (made + BATCH_SIZE / 2).min(count);
        let batch: Vec<_> = (made..end)
            .map(|n| recent_message(n, &account, &inbox, now, &mut rng))
            .collect();
        let written = sql::in_scope(&connection, move |scope| async move {
            let mut written = Vec::with_capacity(batch.len());
            for (message, body) in batch {
                let id = file_message(&scope, message.account_id, message).await;
                write_body(&scope, id, &body).await;
                written.push(id);
            }
            Ok::<_, crate::Error>(written)
        })
        .await
        .expect("commit a batch of recent mail");
        ids.extend(written);
        made = end;
    }
    ids
}

/// One recent message and its body, for [`seed_recent_with_bodies`].
fn recent_message(
    n: usize,
    account: &Account,
    inbox: &Mailbox,
    now: DateTime<Utc>,
    rng: &mut Rng,
) -> (Message, postio_model::MessageBody) {
    let received_at = now - Duration::minutes(i64::from(rng.below(30 * 24 * 60 - 60)));
    let mut message = Message::new(account.id, inbox.id, received_at);
    message.date = Some(received_at);
    let (name, address) = RECENT_SENDERS[rng.below(RECENT_SENDERS.len() as u32) as usize];
    let topic = TOPICS[rng.below(TOPICS.len() as u32) as usize].to_lowercase();
    message.from = vec![EmailAddress::new(Some(name), address)];
    message.to = vec![account.address.clone()];
    message.subject = Some(format!("{topic} #{n}"));
    message.rfc_message_id = Some(RfcMessageId::new(format!("recent-{n}@example.invalid")));
    let reader = account
        .address
        .name
        .as_deref()
        .and_then(|name| name.split_whitespace().next())
        .unwrap_or("there");
    let first = name.split_whitespace().next().unwrap_or(name);
    let pick = |from: &[&str], rng: &mut Rng| {
        from[rng.below(from.len() as u32) as usize].replace("{t}", &topic)
    };
    let lead = if rng.chance(33) {
        pick(RECENT_ASKS, rng)
    } else {
        pick(RECENT_UPDATES, rng)
    };
    let news = pick(RECENT_UPDATES, rng);
    let mut text = format!(
        "Hi {reader},\n\n{lead}\n\n{news}\n\nThanks,\n{first}\n\n-- \n{name}\nExample Co.\n"
    );
    if rng.chance(60) {
        text.push_str(&format!(
            "\nOn Mon, 7 Sep 2026 at 09:12, {reader} <{}> wrote:\n\
             > Could you look at the {topic} when you have a moment?\n\
             > It is the one we spoke about on Friday.\n",
            account.address.address,
        ));
    }
    message.preview = Some(text.chars().take(120).collect());
    message.size = text.len() as u64;
    message.flags = FlagSet::new();
    message.sync.body_state = BodyState::NotFetched;
    let body = postio_model::MessageBody {
        text: Some(text),
        html: None,
    };
    (message, body)
}

/// Who writes the recent mail: invented people, at reserved domains.
const RECENT_SENDERS: &[(&str, &str)] = &[
    ("Quinn Abara", "quinn.abara@example.net"),
    ("Tove Bergstrom", "tove.bergstrom@example.com"),
    ("Yoko Tanaka", "tanaka.yoko@jp.example"),
    ("Remy Okafor", "remy@example.org"),
    ("Ines Varga", "ines.varga@example.test"),
];

/// What a third of the recent mail asks of the reader.
const RECENT_ASKS: &[&str] = &[
    "Can you approve the {t} figures by Friday so finance can close the quarter?",
    "Please leave comments on the {t} by Wednesday; I'd like to freeze it Thursday.",
    "Could you send me the {t} before the end of the week?",
    "Would you review the {t} and let me know what you think?",
    "Are you free to go over the {t} on Tuesday afternoon?",
];

/// What the rest says: news, with nothing asked.
const RECENT_UPDATES: &[&str] = &[
    "Just a note that the {t} went out this morning.",
    "The {t} is in the shared folder now, for when it is useful.",
    "We moved the {t} to next month, so nothing is needed this week.",
    "Everything on the {t} is on track, and the numbers look good.",
];

/// How many messages the next seeded conversation holds.
///
/// Shaped like a working mailbox rather than measured from one: most mail
/// is a message nobody answered, some is an exchange, and a few threads run
/// long enough that paging within one is a real case.
fn conversation_length(rng: &mut Rng) -> usize {
    match rng.below(100) {
        0..60 => 1,
        60..80 => 2,
        80..90 => 3 + rng.below(2) as usize,
        90..97 => 5 + rng.below(5) as usize,
        _ => 10 + rng.below(21) as usize,
    }
}

async fn create_folders(connection: &Connection, account: &Account) -> Vec<Mailbox> {
    let mut folders = Vec::with_capacity(FOLDERS.len());
    for path in FOLDERS {
        folders.push(test_support::mailbox(connection, account, path).await);
    }
    folders
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
async fn load_folders(connection: &Connection, account: &Account) -> Vec<Mailbox> {
    MailboxRepository::new(connection)
        .list_for_account(account.id)
        .await
        .expect("reload seeded mailboxes")
}

/// Groups an already-seeded account's messages into conversations of
/// exactly `per_thread`, replacing the threads they had. Answers how many
/// threads.
///
/// [`seed_large`] threads its messages in mixed lengths already; this is for
/// a benchmark or test that needs one uniform length to reason about.
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
pub async fn thread_seeded_messages(
    database: &Store,
    account: postio_model::AccountId,
    per_thread: usize,
) -> u32 {
    assert!(per_thread > 0, "a conversation holds at least one message");
    thread_in_runs(database, account, move || per_thread).await
}

/// Files an account's messages, newest first, into consecutive
/// conversations whose lengths `length` says, replacing whatever threads it
/// had -- so threading an already-threaded fixture again leaves no empty
/// threads behind. Answers how many threads.
async fn thread_in_runs(
    database: &Store,
    account: postio_model::AccountId,
    mut length: impl FnMut() -> usize + Send + 'static,
) -> u32 {
    let connection = database.connect().await.expect("a checked-out connection");

    // `all_unbounded`: a seeder threading the whole corpus is the shape that
    // exception exists for, and a limit here would leave a fixture partly
    // threaded -- which is worse than slow, because every assertion above it
    // would still pass.
    let rows: Vec<(i64, Option<String>)> = {
        sql::all_unbounded(
            &connection,
            "SELECT id, subject FROM messages WHERE account_id = ?1
              ORDER BY received_at DESC, id DESC",
            [account.get()],
            |row| Ok((row.col::<i64>(0)?, row.col::<Option<String>>(1)?)),
        )
        .await
        .expect("read the seeded message list")
    };

    sql::in_scope(&connection, |scope| async move {
        // Replaced, not added to: `ON DELETE SET NULL` clears the messages'
        // membership with the threads.
        sql::execute(
            &scope,
            "DELETE FROM threads WHERE account_id = ?1",
            [account.get()],
        )
        .await
        .expect("clear the seeded threads");
        let mut threads = 0;
        let mut rest = rows.as_slice();
        while !rest.is_empty() {
            let (chunk, after) = rest.split_at(length().clamp(1, rest.len()));
            rest = after;
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
            sql::execute(
                &scope,
                "INSERT INTO threads (account_id, subject, message_count, unread_count,
                                      has_attachments, is_flagged, first_at, last_at)
                 VALUES (?1, ?2, 0, 0, 0, 0, 0, 0)",
                bind![account.get(), subject],
            )
            .await
            .expect("insert a seeded thread");
            let thread = scope.last_insert_rowid();
            for (id, _) in chunk {
                sql::execute(
                    &scope,
                    "UPDATE messages SET thread_id = ?1 WHERE id = ?2",
                    [thread, *id],
                )
                .await
                .expect("file a seeded message into its thread");
            }
            threads += 1;
        }
        // The aggregates, in one statement rather than per thread.
        sql::execute(
            &scope,
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
        .await
        .expect("recompute the seeded thread aggregates");
        Ok::<_, crate::Error>(threads)
    })
    .await
    .expect("commit the threading batch")
}

/// Inserts `message`, files it into a thread, and remembers who wrote it.
/// Write `body` into `blobs` and point `id` at it.
///
/// Compressed into the row by the repository, the same path real mail takes
/// (ADR 0020).
async fn write_body(connection: &Connection, id: MessageId, body: &postio_model::MessageBody) {
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
                // Seeded mail carries no header block: the fixture's own bytes
                // are parsed for the parts this needs and the block has no
                // reader in a seeded store.
                headers: None,
                headers_truncated: false,
                // The corpus fixtures the seed draws on decode cleanly; the
                // ones that do not are `postio-model`'s to test, and a seeded
                // store claiming a decode problem would put a caveat over
                // demo mail (#901).
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .await
        .expect("store a seeded body");
}

async fn file_message(
    connection: &Connection,
    account_id: postio_model::AccountId,
    mut message: Message,
) -> MessageId {
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("insert a seeded message");
    ThreadingRepository::new(connection, account_id)
        .thread(&message)
        .await
        .expect("thread a seeded message");
    record_correspondents(connection, &message).await;
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
async fn record_correspondents(connection: &Connection, message: &Message) {
    if let Err(error) = ContactRepository::new(connection)
        .record_message(message)
        .await
    {
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

/// Stamps every folder `report` created as synced at `at`.
///
/// A seed has never talked to a server, so every folder says `never
/// synced`; a picture of an ordinary day wants one that has.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub async fn stamp_synced(database: &Store, report: &SeedReport, at: DateTime<Utc>) {
    let connection = database.connect().await.expect("a checked-out connection");
    let repository = MailboxRepository::new(&connection);
    for mailbox in &report.mailboxes {
        let mut mailbox = mailbox.clone();
        mailbox.last_synced_at = Some(at);
        repository
            .update(&mailbox)
            .await
            .expect("stamp a seeded folder");
    }
}

/// Queues one message to send and leaves it unsent: the Outbox's one row
/// (spec 003 FR-012), queued at `at`.
///
/// Through [`DraftRepository::queue_send`](crate::repository::DraftRepository::queue_send),
/// which is what a composer's Send calls, so the row and its count are read
/// back out of the store as they are for a real send. Nothing drains it.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub async fn queue_one_to_send(database: &Store, account: AccountId, at: DateTime<Utc>) {
    let connection = database.connect().await.expect("a checked-out connection");
    let drafts = crate::repository::DraftRepository::new(&connection);
    let mut draft = postio_model::Draft::new(account);
    draft.subject = "Re: maildir index rebuild is O(n²)".to_owned();
    draft.to = vec![EmailAddress::new(Some("Lena Tomlin"), "lena@example.com")];
    draft.body.text = Some("Confirmed on 0.4.1 — sending the trace now.".to_owned());
    drafts.save(&mut draft).await.expect("the draft saves");
    drafts
        .queue_send(&mut draft, at)
        .await
        .expect("the send queues");
}

/// Saves one draft and walks away from it: never queued, so it is a draft
/// someone left rather than a message on its way out.
///
/// # Panics
///
/// If a write fails, as [`seed_small`] does.
pub async fn leave_a_draft(database: &Store, account: AccountId) {
    let connection = database.connect().await.expect("a checked-out connection");
    let drafts = crate::repository::DraftRepository::new(&connection);
    let mut draft = postio_model::Draft::new(account);
    draft.subject = "Notes for Thursday".to_owned();
    draft.to = vec![EmailAddress::new(Some("Nadia Okafor"), "nadia@example.org")];
    draft.body.text = Some("Agenda so far: the index rebuild, then the release.".to_owned());
    drafts.save(&mut draft).await.expect("the draft saves");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repository::ContactRepository;

    #[tokio::test]
    async fn seeding_twice_with_the_same_seed_gives_the_same_store() {
        let first = seed_small(&test_support::memory().await, 7).await;
        let second = seed_small(&test_support::memory().await, 7).await;

        assert_eq!(first.message_count, second.message_count);
        for role in [MailboxRole::Inbox, MailboxRole::Sent, MailboxRole::Archive] {
            assert_eq!(
                first.mailbox(role).map(|m| m.counts),
                second.mailbox(role).map(|m| m.counts),
                "{role:?} disagrees between two seeds with the same key"
            );
        }
    }

    #[tokio::test]
    async fn a_different_seed_gives_a_different_distribution() {
        let a = seed_small(&test_support::memory().await, 1).await;
        let b = seed_small(&test_support::memory().await, 2).await;

        assert_eq!(a.message_count, b.message_count, "same corpus, either way");
        assert_ne!(
            a.mailbox(MailboxRole::Inbox).unwrap().counts,
            b.mailbox(MailboxRole::Inbox).unwrap().counts,
            "different seeds should not land on exactly the same split"
        );
    }

    #[tokio::test]
    async fn every_folder_exists_and_the_cached_counts_are_not_lies() {
        let database = test_support::memory().await;
        let report = seed_small(&database, 3).await;

        assert_eq!(report.mailboxes.len(), FOLDERS.len());
        let total: u32 = report.mailboxes.iter().map(|m| m.counts.total).sum();
        assert_eq!(total as usize, report.message_count);

        let connection = database.connect().await.unwrap();
        for mailbox in &report.mailboxes {
            let actual = MessageRepository::new(&connection)
                .count(&crate::repository::ListQuery::mailbox(mailbox.id))
                .await
                .unwrap();
            assert_eq!(
                actual, mailbox.counts.total,
                "{}'s cached total does not match what is actually there",
                mailbox.path
            );
        }
    }

    #[tokio::test]
    async fn corpus_replies_land_in_the_same_thread_as_their_root() {
        let database = test_support::memory().await;
        let report = seed_small(&database, 11).await;
        let connection = database.connect().await.unwrap();

        let root = test_corpus::load("list-thread-01-root").parse();
        let threading = ThreadingRepository::new(&connection, report.account.id);
        let thread_id = threading
            .thread_of(root.rfc_message_id.as_ref().unwrap())
            .await
            .expect("looking up the root's thread must not fail")
            .expect("the root fixture was seeded and threaded");

        let reply = test_corpus::load("list-thread-02-reply").parse();
        assert_eq!(
            threading
                .thread_of(reply.rfc_message_id.as_ref().unwrap())
                .await
                .expect("looking up the reply's thread must not fail"),
            Some(thread_id),
            "a reply fixture must land in its root's thread"
        );
    }

    #[tokio::test]
    async fn no_message_pretends_to_have_a_body_that_was_never_written() {
        let database = test_support::memory().await;
        let report = seed_small(&database, 4).await;
        let connection = database.connect().await.unwrap();

        for mailbox in &report.mailboxes {
            let rows = MessageRepository::new(&connection)
                .page(&crate::repository::ListQuery::mailbox(mailbox.id).limit(u32::MAX))
                .await
                .unwrap();
            for row in rows {
                let message = MessageRepository::new(&connection)
                    .get(row.id)
                    .await
                    .unwrap()
                    .expect("the row just listed");
                assert_eq!(message.sync.body_state, BodyState::NotFetched);
                assert!(message.raw_blob_id.is_none());
            }
        }
    }

    #[tokio::test]
    async fn the_large_variant_inserts_exactly_as_many_messages_as_asked() {
        let database = test_support::memory().await;
        let report = seed_large(&database, 5, 250).await;

        assert_eq!(report.message_count, 250);
        let total: u32 = report.mailboxes.iter().map(|m| m.counts.total).sum();
        assert_eq!(total, 250);
    }

    #[tokio::test]
    async fn the_large_variant_batches_across_more_than_one_transaction() {
        // A message count that spans several BATCH_SIZE-sized transactions,
        // to prove batching does not drop or duplicate rows at the seam.
        let database = test_support::memory().await;
        let report = seed_large(&database, 9, BATCH_SIZE * 2 + 137).await;

        assert_eq!(report.message_count, BATCH_SIZE * 2 + 137);
        let total: u32 = report.mailboxes.iter().map(|m| m.counts.total).sum();
        assert_eq!(total as usize, report.message_count);
    }

    #[tokio::test]
    async fn a_seeded_store_knows_who_has_written_to_it() {
        // `postio-3ta`. Contacts are recorded by the *sync* path, and a seeded
        // store never goes near it — so every screenshot, demo and test built
        // on one had an `@` palette and a recipient completion that were
        // empty however much mail was in the store. A fixture that models a
        // synced account has to model this too, or the surfaces that read it
        // cannot be told apart from the ones nobody wired up.
        let database = test_support::memory().await;
        let report = seed_small(&database, 7).await;
        let connection = database.connect().await.expect("a checked-out connection");

        let contacts = ContactRepository::new(&connection)
            .search(Some(report.account.id), "", 1_000)
            .await
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
            .await
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

    #[tokio::test]
    async fn the_large_variant_records_its_correspondents_too() {
        // Its senders come from a small fixed pool, so this is a handful of
        // rows however many messages there are — and the benches and paging
        // fixtures built on it look like an account somebody actually uses.
        let database = test_support::memory().await;
        let report = seed_large(&database, 9, 500).await;
        let connection = database.connect().await.expect("a checked-out connection");

        let contacts = ContactRepository::new(&connection)
            .search(Some(report.account.id), "", 1_000)
            .await
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
