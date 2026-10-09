//! The search seed (spec 010 T002): two years of invented mail in the shape
//! of the search design's screens.
//!
//! About five hundred messages in conversations of one to twelve, spread
//! over twenty-four months and ending at the demo's today. The mail the
//! screens show is written out by hand (the Atlas threads, with their
//! senders, labels, files and open asks); the rest is generated from a table
//! of topics so the store is deep enough to page, to facet by month and to
//! rank. Every name is invented, every address is on `example.com`, and
//! the generator is deterministic: the same store every run.

use chrono::{DateTime, Duration, Utc};
use postio_model::{
    AccountId, Attachment, EmailAddress, Flag, FlagSet, Label, LabelId, MailboxId, Message,
    MessageId, RfcMessageId,
};
use postio_storage::repository::{
    ContactRepository, LabelRepository, MarkerRepository, MessageRepository, StoredBody,
    ThreadingRepository,
};
use postio_storage::{BlobStore, Connection, Store};

use crate::files::{SearchFile, search_files};
use crate::{Ask, marker, synced, today};

/// The people of the seed, by index.
const PEOPLE: [(&str, &str); 13] = [
    ("Ada Moreno", "ada@example.com"),
    ("Tomás Reyes", "tomas@example.com"),
    ("Priya Nair", "priya@example.com"),
    ("Ben Adeyemi", "ben@example.com"),
    ("Grace Oyelaran", "grace@example.com"),
    ("Lena Park", "lena@example.com"),
    ("Finance team", "finance@example.com"),
    ("Ravi Iyer", "ravi@example.com"),
    ("Marcus Webb", "marcus@example.com"),
    ("Sofia Lindqvist", "sofia@example.com"),
    ("Hannah Cole", "hannah@example.com"),
    ("Omar Haddad", "omar@example.com"),
    ("Shop receipts", "receipts@example.com"),
];

const ADA: usize = 0;
const TOMAS: usize = 1;
const PRIYA: usize = 2;
const BEN: usize = 3;
const GRACE: usize = 4;
const LENA: usize = 5;
const FINANCE: usize = 6;
const RAVI: usize = 7;
const SHOP: usize = 12;

/// Who a turn is from: a person of [`PEOPLE`], or the account itself.
type From = Option<usize>;

/// A file on a message: a stored blob of [`search_files`] by name, or a
/// part the server has not delivered.
struct Part {
    name: &'static str,
    mime: &'static str,
    /// Its size when there is no stored blob to measure.
    size: u64,
    stored: bool,
}

const fn stored(name: &'static str) -> Part {
    Part {
        name,
        mime: "",
        size: 0,
        stored: true,
    }
}

const fn remote(name: &'static str, size: u64) -> Part {
    Part {
        name,
        mime: "application/pdf",
        size,
        stored: false,
    }
}

/// One conversation to file.
struct Thread {
    subject: String,
    folder: &'static str,
    label: Option<&'static str>,
    /// When its newest message came, before today.
    newest: Duration,
    /// The gap between its messages.
    step: Duration,
    turns: Vec<(From, String)>,
    /// On the newest message.
    parts: Vec<Part>,
    ask: Option<Ask>,
    flagged: bool,
    /// The first message was replied to.
    answered: bool,
    unread: bool,
}

fn days(count: i64) -> Duration {
    Duration::days(count)
}

fn turns(items: &[(From, &str)]) -> Vec<(From, String)> {
    items
        .iter()
        .map(|(from, text)| (*from, (*text).to_owned()))
        .collect()
}

/// The conversations the screens show, written out.
fn curated() -> Vec<Thread> {
    let quiet = Duration::hours(7);
    let thread = |subject: &str, folder, label, newest, turns| Thread {
        subject: subject.to_owned(),
        folder,
        label,
        newest,
        step: quiet,
        turns,
        parts: Vec::new(),
        ask: None,
        flagged: false,
        answered: false,
        unread: false,
    };
    vec![
        Thread {
            parts: vec![stored("Atlas-Q3-budget.xlsx")],
            ask: Some(Ask::Question(
                "Can you approve these by Friday so finance can close the quarter?",
            )),
            answered: true,
            unread: true,
            ..thread(
                "Re: Atlas Q3 budget, final numbers",
                "INBOX",
                Some("Atlas"),
                Duration::minutes(130),
                turns(&[
                    (
                        Some(ADA),
                        "Draft of the Q3 numbers attached to the sheet below; tell me what looks off.",
                    ),
                    (
                        None,
                        "Two lines look high to me. Can you check contractors and tooling?",
                    ),
                    (
                        Some(ADA),
                        "Done. The final Q3 numbers for the Atlas budget are in the attached sheet. Can you approve these by Friday so finance can close the quarter?",
                    ),
                ]),
            )
        },
        Thread {
            flagged: true,
            unread: true,
            ..thread(
                "Atlas staffing plan for Q4",
                "INBOX",
                Some("Atlas"),
                days(1) + Duration::minutes(40),
                turns(&[(
                    Some(TOMAS),
                    "Sharing the draft before Monday's sync. Two more platform roles move the Atlas budget up by about 9% next quarter.",
                )]),
            )
        },
        Thread {
            parts: vec![stored("Atlas-budget-template.xlsx")],
            ..thread(
                "Atlas budget template v2",
                "Archive",
                Some("Atlas"),
                days(55),
                turns(&[(
                    Some(ADA),
                    "The new template is attached. The summary sheet totals the Atlas budget for the year.",
                )]),
            )
        },
        Thread {
            parts: vec![remote("Contractor-invoices-Sep.pdf", 148_000)],
            ..thread(
                "Contractor invoices for September",
                "INBOX",
                None,
                days(3),
                turns(&[(
                    Some(PRIYA),
                    "All three are in the shared folder. Two of them should be charged to the Atlas budget line, not Harbor.",
                )]),
            )
        },
        Thread {
            ask: Some(Ask::Todo {
                days: 2,
                sentence: "Please confirm by 30 Sep.",
            }),
            ..thread(
                "Q3 close: budget owners please review",
                "Archive",
                None,
                days(6),
                turns(&[(
                    Some(FINANCE),
                    "Owners: Atlas (Ravi), Harbor (Lena), Platform (Tomás). Please confirm by 30 Sep.",
                )]),
            )
        },
        thread(
            "Re: Harbor SOW",
            "Archive",
            Some("Harbor"),
            days(9),
            turns(&[
                (
                    Some(BEN),
                    "Legal is still reviewing section 4 of the Harbor SOW.",
                ),
                (
                    None,
                    "Understood. Is the vendor cost still in scope for us?",
                ),
                (
                    Some(BEN),
                    "Not sure yet; I will ask them. Legal agrees to move the vendor cost out of the Atlas budget.",
                ),
                (None, "Good, thanks. Send me the redline when you have it."),
            ]),
        ),
        Thread {
            parts: vec![stored("Atlas-Sep-actuals.pdf")],
            ..thread(
                "Atlas September actuals",
                "Receipts",
                Some("Atlas"),
                days(13),
                turns(&[(
                    Some(ADA),
                    "September actuals are attached. Page 2 has the spend against the plan.",
                )]),
            )
        },
        thread(
            "Design headcount for Atlas",
            "Archive",
            None,
            days(17),
            turns(&[(
                Some(GRACE),
                "The two design roles come out of the Harbor budget, not Atlas.",
            )]),
        ),
        Thread {
            parts: vec![stored("Atlas-budget-memo.docx")],
            ..thread(
                "Memo: the Atlas budget for Q4",
                "Archive",
                Some("Atlas"),
                days(30),
                turns(&[(
                    Some(RAVI),
                    "The memo is attached: what the Atlas budget holds and what moves.",
                )]),
            )
        },
        thread(
            "Re: Atlas offsite",
            "Archive",
            Some("Atlas"),
            days(34),
            turns(&[
                (Some(TOMAS), "Where do we want the offsite this time?"),
                (
                    Some(ADA),
                    "Somewhere close. Ada says the offsite fits inside the Atlas budget if we skip the dinner.",
                ),
                (None, "Skipping the dinner is fine by me."),
                (Some(TOMAS), "Then the venue by the harbor works."),
                (Some(ADA), "Booked."),
            ]),
        ),
        thread(
            "Harbor vs Atlas split for shared tooling",
            "Archive",
            Some("Harbor"),
            days(66),
            turns(&[(
                Some(LENA),
                "Proposing 60/40 between Harbor and the Atlas budget for the CI runners.",
            )]),
        ),
        Thread {
            parts: vec![remote("Invoice-2026-08.pdf", 61_000)],
            ..thread(
                "Invoice 2026-08, Atlas contractor hours",
                "INBOX",
                Some("Atlas"),
                days(20),
                turns(&[(
                    Some(PRIYA),
                    "Attached is the invoice for August: Atlas contractor hours, 212 in all.",
                )]),
            )
        },
        Thread {
            parts: vec![stored("Atlas-budget-review.pptx")],
            ..thread(
                "Atlas budget review deck",
                "Archive",
                Some("Atlas"),
                days(40),
                turns(&[(
                    Some(ADA),
                    "The deck for Thursday's review is attached. Two slides, the second is the quarter's spend.",
                )]),
            )
        },
        Thread {
            parts: vec![stored("atlas-budget-notes.txt")],
            ..thread(
                "Notes from the Atlas budget sync",
                "Archive",
                Some("Atlas"),
                days(22),
                turns(&[
                    (
                        Some(TOMAS),
                        "My notes from the sync are attached as plain text.",
                    ),
                    (None, "Thanks, I will add the contractor numbers tomorrow."),
                ]),
            )
        },
    ]
}

/// A kind of conversation the generator repeats across the two years.
struct Topic {
    subject: &'static str,
    label: Option<&'static str>,
    folder: &'static str,
    people: [usize; 2],
    lines: [&'static str; 4],
    /// Says both words; only the recent year gets these.
    atlas_budget: bool,
}

const TOPICS: [Topic; 15] = [
    Topic {
        subject: "Harbor weekly notes",
        label: Some("Harbor"),
        folder: "INBOX",
        people: [LENA, BEN],
        lines: [
            "Notes from the weekly: the pagination change is in review.",
            "Thanks, I will read the draft tonight.",
            "One open question on the export path; can we settle it Thursday?",
            "Settled. Moving on to the rate limits.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Atlas budget check-in",
        label: Some("Atlas"),
        folder: "INBOX",
        people: [ADA, FINANCE],
        lines: [
            "Quick check-in on the Atlas budget: are we on plan for the quarter?",
            "Close to plan; contractors are the only line running over.",
            "Please send the contractor breakdown when you have it.",
            "Sent. The Atlas budget holds if the two reviews finish on time.",
        ],
        atlas_budget: true,
    },
    Topic {
        subject: "Your coffee order",
        label: Some("Receipts"),
        folder: "Receipts",
        people: [SHOP, SHOP],
        lines: [
            "Thank you for your order. It will be ready for pickup at noon.",
            "Your order has been picked up.",
            "Thanks for ordering again.",
            "Here is your receipt.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Atlas roadmap review",
        label: Some("Atlas"),
        folder: "INBOX",
        people: [TOMAS, RAVI],
        lines: [
            "The roadmap review is Tuesday. Budget is unchanged this quarter.",
            "Noted. The search work stays at the top.",
            "Can you add the migration plan to the agenda?",
            "Added; it goes right after the demo.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Dinner on Saturday",
        label: None,
        folder: "INBOX",
        people: [9, 10],
        lines: [
            "Are you free for dinner on Saturday? We found a place by the water.",
            "Yes, count us in. Should we bring anything?",
            "Just yourselves. Seven o'clock.",
            "See you there.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Budget approvals",
        label: None,
        folder: "INBOX",
        people: [FINANCE, 8],
        lines: [
            "Budget approvals are due on the 28th. Please review your lines.",
            "Reviewed. One correction on travel.",
            "Applied. Thank you.",
            "Closing the round now.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Design review",
        label: Some("Harbor"),
        folder: "INBOX",
        people: [GRACE, LENA],
        lines: [
            "The design review is Friday at ten. The agenda is navigation and empty states.",
            "I will bring the prototype.",
            "Please share it a day ahead so we can read it.",
            "Shared in the usual folder.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Atlas onboarding",
        label: Some("Atlas"),
        folder: "INBOX",
        people: [ADA, 11],
        lines: [
            "Welcome to Atlas. Your accounts are ready and the checklist is attached below.",
            "Thanks, everything works. One question about repository access.",
            "Granted just now.",
            "Confirmed, I can push.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Invoice for the month",
        label: Some("Receipts"),
        folder: "Receipts",
        people: [PRIYA, SHOP],
        lines: [
            "Attached is this month's invoice. The total is unchanged.",
            "Received, thank you.",
            "Payment is scheduled for the end of the month.",
            "Paid.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Atlas budget, contractor hours",
        label: Some("Atlas"),
        folder: "INBOX",
        people: [PRIYA, ADA],
        lines: [
            "Contractor hours for the month are in. They count against the Atlas budget.",
            "Thanks. Please keep the Harbor hours on their own line.",
            "Done; the split is in the sheet.",
            "Approved.",
        ],
        atlas_budget: true,
    },
    Topic {
        subject: "Travel plans",
        label: None,
        folder: "INBOX",
        people: [10, 9],
        lines: [
            "Flights are booked for the conference. The hotel is next.",
            "Great. Can we get the one near the venue?",
            "It is the same one as last year.",
            "Booked.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Harbor API questions",
        label: Some("Harbor"),
        folder: "INBOX",
        people: [BEN, RAVI],
        lines: [
            "Two questions on the Harbor API: pagination and rate limits.",
            "Cursor pagination, and ten requests a second per key.",
            "That works for us. Thanks.",
            "Added to the docs.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Quarterly planning",
        label: None,
        folder: "INBOX",
        people: [TOMAS, FINANCE],
        lines: [
            "Planning for the quarter starts next week. Please send your top three asks.",
            "Mine are search, sync reliability and the new composer.",
            "Thanks, collected.",
            "The plan is in the shared folder.",
        ],
        atlas_budget: false,
    },
    Topic {
        subject: "Atlas budget for the offsite",
        label: Some("Atlas"),
        folder: "INBOX",
        people: [RAVI, ADA],
        lines: [
            "Can the offsite come out of the Atlas budget this time?",
            "If we keep it to two days, yes.",
            "Two days it is.",
            "I will send the invitations.",
        ],
        atlas_budget: true,
    },
    Topic {
        subject: "Reading list",
        label: None,
        folder: "INBOX",
        people: [9, 8],
        lines: [
            "This month's reading list is short: two papers and a talk.",
            "I will take the talk.",
            "The papers are in the folder.",
            "Thanks, finished the first one.",
        ],
        atlas_budget: false,
    },
];

/// Conversation sizes the generator cycles through: ones, twos, a long one.
const SIZES: [usize; 16] = [1, 2, 1, 3, 1, 4, 2, 1, 5, 1, 2, 3, 12, 1, 2, 6];

/// How many generated conversations there are.
const GENERATED: usize = 150;

fn generated() -> Vec<Thread> {
    let mut threads = Vec::with_capacity(GENERATED);
    for index in 0..GENERATED {
        // Two weeks back to about twenty-four months, a conversation every
        // four and a half days or so, at a different hour each time.
        let age = 14 + (index as i64) * 47 / 10;
        let newest = days(age) + Duration::minutes(((index as i64) * 397) % 600);
        let mut pick = (index * 7) % TOPICS.len();
        while TOPICS[pick].atlas_budget && age > 365 {
            pick = (pick + 1) % TOPICS.len();
        }
        let topic = &TOPICS[pick];
        let size = SIZES[index % SIZES.len()];
        let when = today() - newest;
        let folder = if topic.folder == "INBOX" && age > 90 {
            "Archive"
        } else {
            topic.folder
        };
        let items = (0..size)
            .map(|turn| {
                let from = if turn % 2 == 0 {
                    Some(topic.people[(turn / 2) % 2])
                } else {
                    None
                };
                (from, topic.lines[turn % topic.lines.len()].to_owned())
            })
            .collect();
        threads.push(Thread {
            subject: format!("{}, {}", topic.subject, when.format("%b %Y")),
            folder,
            label: topic.label,
            newest,
            step: Duration::hours(18),
            turns: items,
            parts: Vec::new(),
            ask: None,
            flagged: index % 23 == 5,
            answered: size > 1 && index % 3 == 0,
            unread: age < 20 && index % 3 == 0,
        });
    }
    threads
}

fn address(from: From) -> EmailAddress {
    match from {
        Some(person) => EmailAddress::new(Some(PEOPLE[person].0), PEOPLE[person].1),
        None => EmailAddress::new(Some("You"), "you@example.com"),
    }
}

/// Stores every [`search_files`] blob into `blobs` and names each one.
///
/// Blob ids are keyed digests of the bytes, so a store opened under the
/// test keys (`postio_storage::test_support::blob_keys`) answers with the
/// ids the seeded attachment rows carry. A host over [`seeded`](crate::seeded)'s
/// store calls this on the blob store it starts with.
///
/// # Errors
///
/// The store's: a blob that could not be written.
pub fn store_search_blobs(
    blobs: &BlobStore,
) -> postio_storage::Result<Vec<(&'static str, postio_model::BlobId)>> {
    search_files()
        .into_iter()
        .map(|file| Ok((file.name, blobs.put(&file.bytes)?)))
        .collect()
}

/// The search seed's store and its account.
pub async fn search_demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
    let mut folders: Vec<(&'static str, MailboxId)> = vec![("INBOX", inbox)];
    for folder in ["Archive", "Receipts"] {
        let made = postio_storage::test_support::mailbox(&connection, &account, folder).await;
        folders.push((folder, made.id));
    }
    let today = today();
    synced(&connection, inbox, today).await;

    // The bytes' ids, from a scratch blob store opened under the test keys.
    let scratch = tempfile::tempdir().expect("scratch for the blobs");
    let blobs = BlobStore::open(
        scratch.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    let stored = store_search_blobs(&blobs).expect("the seed's files");
    let files = search_files();

    let mut labels: std::collections::HashMap<&str, LabelId> = std::collections::HashMap::new();
    let mut bodies: Vec<(MessageId, String)> = Vec::new();
    let mut threads = curated();
    threads.extend(generated());
    for (index, thread) in threads.iter().enumerate() {
        let mailbox = folders
            .iter()
            .find(|(name, _)| *name == thread.folder)
            .map_or(inbox, |(_, id)| *id);
        let label = match thread.label {
            Some(name) => Some(match labels.get(name) {
                Some(id) => *id,
                None => {
                    let mut label = Label::new(account.id, name);
                    LabelRepository::new(&connection)
                        .create(&mut label)
                        .await
                        .expect("a label");
                    labels.insert(name, label.id);
                    label.id
                }
            }),
            None => None,
        };
        file_thread(
            &connection,
            account.id,
            mailbox,
            label,
            index,
            thread,
            today,
            &files,
            &stored,
            &mut bodies,
        )
        .await;
    }
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the search index");
    // Indexed as the body indexer would have by now: a seed whose bodies
    // only the subject search can see shows no body passage and no quoted
    // text until a background pass runs. After `ensure_schema`, whose first
    // run empties the body index it creates, and in one transaction.
    postio_storage::sql::batch(&connection, "BEGIN")
        .await
        .expect("begin the bodies");
    for (message, body) in &bodies {
        postio_index::index::index_body(&connection, message.get(), Some(body))
            .await
            .expect("a body indexed");
    }
    postio_storage::sql::batch(&connection, "COMMIT")
        .await
        .expect("commit the bodies");
    drop(connection);
    (database, account.id)
}

#[allow(clippy::too_many_arguments)]
async fn file_thread(
    connection: &Connection,
    account: AccountId,
    mailbox: MailboxId,
    label: Option<LabelId>,
    index: usize,
    thread: &Thread,
    today: DateTime<Utc>,
    files: &[SearchFile],
    stored: &[(&'static str, postio_model::BlobId)],
    bodies: &mut Vec<(MessageId, String)>,
) {
    let count = thread.turns.len();
    let correspondent = thread
        .turns
        .iter()
        .find_map(|(from, _)| *from)
        .unwrap_or(ADA);
    let mut previous: Option<RfcMessageId> = None;
    let mut last = MessageId::UNASSIGNED;
    let mut before: Option<(From, String, DateTime<Utc>)> = None;
    for (turn, (from, text)) in thread.turns.iter().enumerate() {
        let newest = turn + 1 == count;
        let at = today - thread.newest - thread.step * (count - 1 - turn) as i32;
        let mut message = Message::new(account, mailbox, at);
        message.date = Some(at);
        message.from = vec![address(*from)];
        message.to = vec![if from.is_some() {
            address(None)
        } else {
            address(Some(correspondent))
        }];
        message.subject = Some(if turn == 0 || thread.subject.starts_with("Re: ") {
            thread.subject.clone()
        } else {
            format!("Re: {}", thread.subject)
        });
        let body = match &before {
            None => format!("{text}\n"),
            Some((who, quoted, when)) => {
                let name = who.map_or("You", |person| PEOPLE[person].0);
                let quote: String = quoted.lines().map(|line| format!("> {line}\n")).collect();
                format!(
                    "{text}\n\nOn {}, {name} wrote:\n{quote}",
                    when.format("%a %-d %b %Y at %H:%M")
                )
            }
        };
        message.preview = Some(text.chars().take(110).collect());
        let mut flags = FlagSet::new();
        if !(newest && thread.unread) {
            flags.insert(Flag::Seen);
        }
        if turn == 0 && thread.answered {
            flags.insert(Flag::Answered);
        }
        if newest && thread.flagged {
            flags.insert(Flag::Flagged);
        }
        message.flags = flags;
        if newest {
            for (position, part) in thread.parts.iter().enumerate() {
                let file = part
                    .stored
                    .then(|| files.iter().find(|file| file.name == part.name))
                    .flatten();
                let (mime, size) = file.map_or((part.mime, part.size), |file| {
                    (file.mime, file.bytes.len() as u64)
                });
                let mut attachment = Attachment::new(MessageId::UNASSIGNED, mime, size);
                attachment.filename = Some(part.name.to_owned());
                attachment.part_id = Some((position + 2).to_string());
                if file.is_some() {
                    attachment.blob_id = stored
                        .iter()
                        .find(|(name, _)| *name == part.name)
                        .map(|(_, id)| id.clone());
                }
                message.attachments.push(attachment);
            }
        }
        let id = RfcMessageId::new(format!("<search.{index}.{turn}@example.test>"));
        message.rfc_message_id = Some(id.clone());
        if let Some(parent) = previous.replace(id) {
            message.in_reply_to = Some(parent.clone());
            message.references = vec![parent];
        }
        MessageRepository::new(connection)
            .create(&mut message)
            .await
            .expect("a message");
        ContactRepository::new(connection)
            .record_message(&message)
            .await
            .expect("its correspondents");
        ThreadingRepository::new(connection, account)
            .thread(&message)
            .await
            .expect("threaded");
        bodies.push((message.id, body.clone()));
        MessageRepository::new(connection)
            .set_body(
                message.id,
                &StoredBody {
                    text: Some(body),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
        if let Some(label) = label {
            LabelRepository::new(connection)
                .attach(message.id, label)
                .await
                .expect("labelled");
        }
        before = Some((*from, text.clone(), at));
        last = message.id;
    }
    if let Some(ask) = &thread.ask {
        MarkerRepository::new(connection)
            .insert(&marker(last, ask, today))
            .await
            .expect("a marker");
    }
}
