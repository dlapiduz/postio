//! The demo store and window Focus's `shot` and its storyboard runner share.
//!
//! `examples/shot.rs` and the storyboard runner both need a real
//! [`FocusWindow`](crate::window::FocusWindow) over a real, migrated,
//! seeded store, started and adopted the way the application starts it. That
//! setup lives here so the two share it rather than each keeping a copy
//! (specs/008-storyboards R11); argument parsing, staging a numbered screen
//! and the pictures stay with the tools.
//!
//! Behind the `demo` feature, which turns on `postio-storage/test-support`:
//! the seeds and the in-memory store are test-support code and must not
//! reach a normal build.
//!
//! The store is today's inbox in the references' shape, over the storage
//! seed. Every name is invented and every address is on a reserved domain.
//! Nothing touches the network.

#![allow(missing_docs)]

pub mod storyboard;

/// A condition of the store, named as every app names it
/// (specs/008-storyboards R11): something no step can produce, such as
/// thirty conversations or a draft left over from yesterday.
///
/// The bases decide what mail the store holds; the rest add one thing to
/// [`Seed::Small`]'s inbox. `first-run` is not here: the classic app's
/// orientation strip it showed is one Focus dropped (`classic-parity.md`
/// row 12), and Focus's own first run is the store with no account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    /// Today's inbox over the storage seed: the default.
    Small,
    /// An account with its folders and no mail in the inbox.
    Empty,
    /// The small seed with the row screen 04 opens refiled as a long
    /// newsletter that paints its own page: a message tall enough to
    /// scroll, with a treatment `O` can switch.
    LongNewsletter,
    /// One conversation of seven messages, the last two unread.
    LongThread,
    /// Thirty-six conversations in the inbox.
    ThirtyThreads,
    /// The small seed and a second account beside the first.
    TwoAccounts,
    /// The small seed and one message queued to send, never sent.
    Outbox,
    /// The small seed and one draft saved and walked away from.
    DraftLeftOver,
    /// The small seed with a backfill in flight, said through the host's
    /// events as the engine says it.
    Backfilling,
}

impl Seed {
    /// Every seed Focus can build.
    pub const ALL: [Seed; 9] = [
        Seed::Small,
        Seed::Empty,
        Seed::LongNewsletter,
        Seed::LongThread,
        Seed::ThirtyThreads,
        Seed::TwoAccounts,
        Seed::Outbox,
        Seed::DraftLeftOver,
        Seed::Backfilling,
    ];

    /// The name a storyboard uses.
    pub fn id(self) -> &'static str {
        match self {
            Seed::Small => "small",
            Seed::Empty => "empty",
            Seed::LongNewsletter => "long-newsletter",
            Seed::LongThread => "long-thread",
            Seed::ThirtyThreads => "thirty-threads",
            Seed::TwoAccounts => "two-accounts",
            Seed::Outbox => "outbox",
            Seed::DraftLeftOver => "draft-left-over",
            Seed::Backfilling => "backfilling",
        }
    }

    /// The seed a storyboard's name stands for.
    pub fn from_id(id: &str) -> Option<Seed> {
        Seed::ALL.into_iter().find(|seed| seed.id() == id)
    }
}

/// A store seeded as `seed` asks, and the account its mail is in.
///
/// The store half only: a backfill in flight is an event, which
/// [`storyboard`] says once the host is up.
pub async fn seeded(seed: Seed) -> (Store, AccountId) {
    match seed {
        Seed::Empty => empty_demo().await,
        Seed::LongThread | Seed::ThirtyThreads => {
            let shape = if seed == Seed::LongThread {
                postio_storage::seed::LONG_THREAD
            } else {
                postio_storage::seed::THIRTY_THREADS
            };
            let database = postio_storage::test_support::memory().await;
            let report = postio_storage::seed::seed_conversations(&database, 11, shape).await;
            postio_storage::seed::stamp_synced(&database, &report, today()).await;
            let connection = database.connect().await.expect("a connection");
            postio_index::index::ensure_schema(&connection)
                .await
                .expect("the search index");
            drop(connection);
            (database, report.account.id)
        }
        Seed::Small
        | Seed::LongNewsletter
        | Seed::TwoAccounts
        | Seed::Outbox
        | Seed::DraftLeftOver
        | Seed::Backfilling => {
            let (database, account) = demo().await;
            match seed {
                Seed::LongNewsletter => treatment_demo(&database, account, "30").await,
                Seed::TwoAccounts => {
                    let second = postio_storage::seed::seed_extra_account(
                        &database,
                        "Home",
                        "home@example.net",
                        12,
                    )
                    .await;
                    postio_storage::seed::stamp_synced(&database, &second, today()).await;
                }
                Seed::Outbox => {
                    postio_storage::seed::queue_one_to_send(&database, account, today()).await;
                }
                Seed::DraftLeftOver => {
                    postio_storage::seed::leave_a_draft(&database, account).await;
                }
                _ => {}
            }
            (database, account)
        }
    }
}

use std::collections::HashMap;

use chrono::{DateTime, Local, TimeZone, Utc};
use gtk::prelude::*;
use postio_model::listing::MarkerKind;
use postio_model::{
    AccountId, Attachment, EmailAddress, Flag, FlagSet, Label, MailboxId, MailboxRole, Message,
    MessageId, RfcMessageId,
};
use postio_storage::repository::{
    ContactRepository, LabelRepository, Marker, MarkerRepository, MarkerSource, MessageRepository,
    ThreadingRepository,
};
use postio_storage::{BlobStore, Store};

use crate::startup::Session;
use crate::window::FocusWindow;

/// This week's newsletters, held and delivered as screen 01's digest row:
/// who, about what, and how many minutes before 16:09 each came.
pub const NEWSLETTERS: &[((&str, &str), &str, i64)] = &[
    (("Ledger", "news@ledger.example"), "The rate decision", 1500),
    (("Ledger", "news@ledger.example"), "Rail funding vote", 4400),
    (("Soil Weekly", "hello@soil.example"), "Fall planting", 2900),
    (
        ("Crate Notes", "notes@crate.example"),
        "CRDT libraries compared",
        3600,
    ),
    (("Tide Tables", "tides@tide.example"), "October tides", 5200),
    (
        ("Harbor Digest", "digest@harbor.example"),
        "Lisbon, again",
        6100,
    ),
];

/// Screen 21's filtered mail: the sender, the subject, the first line, the
/// reason and its source, and how many minutes before 16:09 it was filed.
pub type Filtered = (
    (&'static str, &'static str),
    &'static str,
    &'static str,
    &'static str,
    Option<&'static str>,
    i64,
);
pub const FILTERED: &[Filtered] = &[
    (
        ("Forge", "notifications@forge.example"),
        "[postio/engine] Review requested: search index rebuild (#412)",
        "grace-o requested your review on this pull request.",
        "notification",
        Some("Forge"),
        2,
    ),
    (
        ("Harbor CI", "ci@harbor.example"),
        "main: build 1182 passed",
        "All 3 jobs passed in 6m 12s.",
        "notification",
        Some("CI"),
        6,
    ),
    (
        ("Outdoor Supply", "deals@outdoor.example"),
        "End-of-season sale: 30% off tents",
        "Through Sunday only. Free shipping over $50.",
        "promotion",
        None,
        11,
    ),
    (
        ("Pantry Co.", "orders@pantry.example"),
        "Your order 4821 has been delivered",
        "Left at the front door at 15:41.",
        "shipping",
        None,
        25,
    ),
    (
        ("Forge", "notifications@forge.example"),
        "[postio/engine] Merged: IMAP IDLE reconnect (#409)",
        "lena-p merged 3 commits into main.",
        "notification",
        Some("Forge"),
        39,
    ),
    (
        ("Calendar", "calendar@calendar.example"),
        "Updated: Harbor design review",
        "The time of this event changed to Tue 29 Sep 10:00.",
        "notification",
        Some("Calendar"),
        57,
    ),
    (
        ("Account Security Team", "security@verify.example"),
        "Your mailbox is almost full, verify now",
        "Click here within 24 hours to keep your account active.",
        "spam",
        None,
        72,
    ),
    (
        ("Metro Power", "billing@power.example"),
        "Payment received, thank you",
        "We received your payment of $142.17.",
        "receipt",
        None,
        89,
    ),
    (
        ("Social Circle", "notify@social.example"),
        "Rita mentioned you in a comment",
        "See what Rita said about your post.",
        "social",
        None,
        107,
    ),
];

/// What a row's second line says, when it has one.
pub enum Ask {
    /// An invitation, starting this many days from today at this hour, for
    /// this many minutes.
    Invite { days: i64, hour: i64, minutes: i64 },
    /// A question, quoting this sentence.
    Question(&'static str),
    /// A to-do, due this many days from today, quoting this sentence.
    Todo { days: i64, sentence: &'static str },
}

/// One of today's conversations.
pub struct Row {
    name: &'static str,
    address: &'static str,
    subject: &'static str,
    preview: &'static str,
    /// Minutes before 16:09 today.
    minutes: i64,
    unread: bool,
    labels: &'static [&'static str],
    attachment: bool,
    /// How many messages the conversation holds.
    messages: usize,
    ask: Option<Ask>,
}

/// Today's inbox, top to bottom, in the references' shape.
pub const TODAY: &[Row] = &[
    Row {
        name: "Hollis Varga",
        address: "hollis@example.com",
        subject: "Invitation: Harbor design review",
        preview: "Tuesday 10:00\u{2013}10:45, Room 3B. Agenda: navigation, empty states, the export flow.",
        minutes: 7,
        unread: true,
        labels: &["Harbor"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Invite {
            days: 3,
            hour: 10,
            minutes: 45,
        }),
    },
    Row {
        name: "Marisol Quint",
        address: "marisol@example.com",
        subject: "Re: Atlas Q3 budget, final numbers",
        preview: "The final Q3 numbers are in the attached sheet. Can you approve these by Friday so finance can close the quarter?",
        minutes: 18,
        unread: true,
        labels: &["Atlas"],
        attachment: true,
        messages: 3,
        ask: Some(Ask::Question(
            "Can you approve these by Friday so finance can close the quarter?",
        )),
    },
    Row {
        name: "Tobias Wren",
        address: "tobias@example.net",
        subject: "Atlas staffing plan for Q4",
        preview: "Sharing the draft before Monday's sync. Nothing needed yet, just a heads up that the platform numbers moved.",
        minutes: 29,
        unread: true,
        labels: &["Atlas"],
        attachment: false,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Juno Castellane",
        address: "juno@example.org",
        subject: "Harbor API draft v3",
        preview: "Uploaded v3 with the pagination changes. Please leave comments by Wednesday; I'd like to freeze it Thursday.",
        minutes: 47,
        unread: true,
        labels: &["Harbor"],
        attachment: false,
        messages: 6,
        ask: Some(Ask::Todo {
            days: 4,
            sentence: "Please leave comments by Wednesday",
        }),
    },
    Row {
        name: "Idris Mallory",
        address: "idris@example.com",
        subject: "Re: Harbor SOW",
        preview: "Legal is still reviewing section 4. Should have it back to you early next week.",
        minutes: 71,
        unread: false,
        labels: &["Harbor"],
        attachment: false,
        messages: 2,
        ask: None,
    },
    Row {
        name: "Pim Aldana",
        address: "pim@example.net",
        subject: "Cabinet order: please sign",
        preview: "Attached the final order for the uppers and the pantry unit. Once you sign I can place it with the supplier.",
        minutes: 98,
        unread: false,
        labels: &["Kitchen reno", "Home"],
        attachment: true,
        messages: 4,
        ask: None,
    },
    Row {
        name: "Solveig Brandt",
        address: "solveig@example.org",
        subject: "Contractor invoices for September",
        preview: "All three are in the shared folder. Two of them are already approved on my side.",
        minutes: 119,
        unread: false,
        labels: &["Atlas"],
        attachment: false,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Caspian Holt",
        address: "caspian@example.com",
        subject: "Intro: Harbor data vendor",
        preview: "A colleague suggested I reach out. We help teams move analytics workloads off the warehouse.",
        minutes: 142,
        unread: true,
        labels: &["Harbor"],
        attachment: false,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Renata Obi",
        address: "renata@example.net",
        subject: "Re: dinner Saturday?",
        preview: "We're in! Should we bring anything? The kids can bring the card game they got last week.",
        minutes: 159,
        unread: true,
        labels: &["Friends"],
        attachment: false,
        messages: 2,
        ask: Some(Ask::Question("Should we bring anything?")),
    },
    Row {
        name: "Northfield Elementary",
        address: "office@northfield.example",
        subject: "Field-trip permission form",
        preview: "Please sign and return the attached form by Monday. The trip to the science museum is next week.",
        minutes: 194,
        unread: true,
        labels: &["Kids"],
        attachment: true,
        messages: 1,
        ask: Some(Ask::Todo {
            days: 2,
            sentence: "Please sign and return the attached form by Monday.",
        }),
    },
    Row {
        name: "Marisol Quint",
        address: "marisol@example.com",
        subject: "Atlas headcount numbers",
        preview: "Quick one: do you have the Q4 headcount numbers from the planning doc? No rush, next week is fine.",
        minutes: 229,
        unread: true,
        labels: &["Atlas"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Question(
            "do you have the Q4 headcount numbers from the planning doc?",
        )),
    },
    Row {
        name: "Anselm Kade",
        address: "anselm@example.org",
        subject: "Re: talk proposal, local-first mail",
        preview: "Thanks for sending this over. The committee meets next week; I'll let you know either way.",
        minutes: 261,
        unread: false,
        labels: &["Talks"],
        attachment: false,
        messages: 2,
        ask: None,
    },
    Row {
        name: "Oak Hill HOA",
        address: "board@oakhill.example",
        subject: "October meeting agenda",
        preview: "Items: pool closing, fall cleanup day, the parking proposal. Minutes from September attached.",
        minutes: 279,
        unread: false,
        labels: &["Home"],
        attachment: true,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Tobias Wren",
        address: "tobias@example.net",
        subject: "Re: 1:1 notes",
        preview: "Added my notes under yours. The main thing for me is the on-call rotation for November.",
        minutes: 307,
        unread: false,
        labels: &["Atlas"],
        attachment: false,
        messages: 3,
        ask: None,
    },
    Row {
        name: "Hollis Varga",
        address: "hollis@example.com",
        subject: "Re: export flow edge cases",
        preview: "I think we can drop the CSV option if the markdown export covers tables. I'd rather ship fewer formats.",
        minutes: 329,
        unread: false,
        labels: &["Harbor"],
        attachment: false,
        messages: 5,
        ask: None,
    },
    Row {
        name: "Mae Sorensen",
        address: "mae@example.net",
        subject: "Photos from the hike",
        preview: "Uploaded the good ones. The waterfall came out well, see the second album.",
        minutes: 357,
        unread: false,
        labels: &["Friends"],
        attachment: true,
        messages: 1,
        ask: None,
    },
    Row {
        name: "Pim Aldana",
        address: "pim@example.net",
        subject: "Schedule for next week",
        preview: "Crew arrives Monday 8 am for the demo of the old uppers. Please clear the counters by Sunday night.",
        minutes: 374,
        unread: true,
        labels: &["Kitchen reno", "Home"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Todo {
            days: 1,
            sentence: "Please clear the counters by Sunday night.",
        }),
    },
    Row {
        name: "Rhea Okonjo",
        address: "rhea@example.org",
        subject: "Coffee next week",
        preview: "I'm in town Tuesday through Thursday. Any morning work for you?",
        minutes: 408,
        unread: true,
        labels: &["Friends"],
        attachment: false,
        messages: 1,
        ask: Some(Ask::Question("Any morning work for you?")),
    },
];

/// The demo's config.toml: filtering on, and one digest rule.
pub const CONFIG: &str = "[focus]\nfiltering = true\n\n[[focus.digests]]\nname = \"Newsletters\"\n\
match = [\"from:news@ledger.example\", \"from:hello@soil.example\", \"from:notes@crate.example\", \"from:tides@tide.example\", \"from:digest@harbor.example\"]\ncadence = \"weekly\"\nday = \"saturday\"\nat = \"16:00\"\n\n\
[filters.waiting]\nquery = \"from:juno\"\npinned = true\norder = 1\nname = \"Waiting on reply\"\n\n\
[filters.atlas]\nquery = \"subject:atlas\"\npinned = true\norder = 2\nname = \"Atlas\"\n\n\
[filters.receipts]\nquery = \"in:Receipts\"\npinned = true\norder = 3\nname = \"Receipts this month\"\n\n\
[filters.school]\nquery = \"from:northfield\"\npinned = true\norder = 4\nname = \"From school\"\n";

/// Screen 25's vault: three projects in `Projects/`, each with open tasks,
/// and the tasks note.
pub fn demo_vault() -> std::io::Result<tempfile::TempDir> {
    let vault = tempfile::tempdir()?;
    let projects = vault.path().join("Projects");
    std::fs::create_dir(&projects)?;
    for (name, open) in [("Harbor", 12), ("Atlas", 9), ("Kitchen reno", 4)] {
        let mut note = format!("# {name}\n\n");
        for n in 1..=open {
            note.push_str(&format!(
                "- [ ] Step {n} [\u{2709}](postio://message/{n})\n"
            ));
        }
        std::fs::write(projects.join(format!("{name}.md")), note)?;
    }
    std::fs::write(vault.path().join("Tasks.md"), "# Tasks\n")?;
    Ok(vault)
}

/// What screens 05 and 06 add to the demo store: the people the To field
/// completes to, and the Harbor thread's recipients and body, so a reply
/// to all has someone to copy and something to quote.
pub async fn compose_demo(database: &Store, account: AccountId) {
    let connection = database.connect().await.expect("a connection");
    let contacts = postio_storage::repository::ContactRepository::new(&connection);
    for (name, address) in [
        ("Grace Oyelaran", "grace@example.org"),
        ("Graham Ellis", "graham@example.net"),
        ("Ada Moreno", "ada@example.com"),
    ] {
        contacts
            .create(
                Some(account),
                &EmailAddress::new(Some(name), address),
                Some(name),
            )
            .await
            .expect("a contact");
    }
    let messages = MessageRepository::new(&connection);
    let newest = RfcMessageId::new(format!(
        "<demo.{HARBOR}.{}@example.test>",
        TODAY[HARBOR as usize].messages - 1
    ));
    let Some(id) = messages
        .ids_by_rfc_message_id(account, &newest)
        .await
        .expect("a lookup")
        .first()
        .copied()
    else {
        return;
    };
    if let Some(mut message) = messages.get(id).await.expect("a read") {
        message.to = vec![
            EmailAddress::new(Some("Test User"), "test@example.com"),
            EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net"),
        ];
        message.cc = vec![
            EmailAddress::new(Some("Grace Oyelaran"), "grace@example.org"),
            EmailAddress::new(None::<String>, "harbor-api@example.org"),
        ];
        messages.update(&mut message).await.expect("its recipients");
    }
    messages
        .set_body(
            id,
            &postio_storage::repository::StoredBody {
                text: Some(TODAY[HARBOR as usize].preview.to_owned()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .expect("its body");
}

/// 16:09 today, local, by the interface's clock: when the references were
/// drawn, and a fixed instant once a storyboard has frozen the clock.
pub fn today() -> DateTime<Utc> {
    postio_ui::clock::now()
        .date_naive()
        .and_hms_opt(16, 9, 0)
        .and_then(|at| Local.from_local_datetime(&at).single())
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or_else(|| postio_ui::clock::now().to_utc())
}

/// Say the inbox last synced at `at`, as a completed pass would.
pub async fn synced(connection: &postio_storage::Connection, inbox: MailboxId, at: DateTime<Utc>) {
    let mailboxes = postio_storage::repository::MailboxRepository::new(connection);
    if let Ok(Some(mut mailbox)) = mailboxes.get(inbox).await {
        mailbox.last_synced_at = Some(at);
        mailboxes
            .update(&mailbox)
            .await
            .expect("the inbox's sync time");
    }
}

/// The empty inbox's store (screen 16): an account and its folders, no mail
/// in the inbox, and a morning's worth of mail filed away.
pub async fn empty_demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let connection = database.connect().await.expect("a connection");
    let (account, inbox) = postio_storage::test_support::account_with_inbox(&connection).await;
    for folder in ["Archive", "Sent", "Drafts", "Trash"] {
        postio_storage::test_support::mailbox(&connection, &account, folder).await;
    }
    let filtered = postio_storage::test_support::mailbox(&connection, &account, "Filtered").await;
    let morning = today() - chrono::Duration::hours(7);
    for step in 0..186_i64 {
        let at = morning + chrono::Duration::minutes(step * 2);
        let mut message = Message::new(account.id, filtered.id, at);
        message.flags = [Flag::Seen].into_iter().collect();
        MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a filtered message");
        postio_storage::repository::FilterDecisionRepository::new(&connection)
            .record(&postio_storage::repository::FilterDecision {
                message: message.id,
                reason: postio_storage::repository::FilterReason::Promotion,
                source: None,
                layer: postio_storage::repository::FilterLayer::Header,
                decided_at: at,
            })
            .await
            .expect("a decision");
    }
    synced(&connection, inbox, today()).await;
    drop(connection);
    (database, account.id)
}

/// The row screen 04 opens: the to-do about the API draft, by its place in
/// [`TODAY`].
pub const OPENED: usize = 3;

/// Its body: the marked sentence, a list, and the quoted history folded
/// under it, as screen 04 draws them.
pub const OPENED_BODY: &str = "Hi all,\n\n\
Uploaded v3 of the Harbor API draft with the pagination changes. The main differences from v2:\n\n\
- Cursor pagination on every list endpoint, replacing page and offset.\n\
- Rate-limit headers are documented for every response.\n\
- /exports moved under /v1/accounts/{id}, as suggested.\n\n\
Please leave comments by Wednesday; I'd like to freeze it Thursday so the client work can start.\n\n\
The rendered PDF and the raw OpenAPI file are attached.\n\n\
Juno\n\n\
On Monday, Hollis Varga wrote:\n\
> Thanks for v2. Two things before the next round:\n\
> the exports path, and the rate limits.\n\
> Everything else reads well to me.\n";

/// Its attachments: the rendered draft and its source.
pub fn opened_parts() -> Vec<Attachment> {
    [
        ("Harbor-API-v3.pdf", "application/pdf", 212_000),
        ("harbor-openapi.yaml", "application/yaml", 38_000),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (name, mime, size))| {
        let mut part = Attachment::new(MessageId::UNASSIGNED, mime, size);
        part.filename = Some(name.to_owned());
        // After the text part, as a synced message's parts are numbered.
        part.part_id = Some((index + 2).to_string());
        part
    })
    .collect()
}

/// Screens 27-29 (the message dialog redesign, T214): the row screen 04
/// opens, refiled as one of the handoff's two HTML bodies from the corpus --
/// the newsletter that paints its own page, or the office mail in black
/// text with the question its card quotes. The row keeps its place, so the
/// screens open it the way screen 04 does.
pub async fn treatment_demo(database: &Store, account: AccountId, screen: &str) {
    let newsletter = matches!(screen, "27" | "30" | "31");
    let connection = database.connect().await.expect("a connection");
    let messages = MessageRepository::new(&connection);
    let newest = RfcMessageId::new(format!(
        "<demo.{OPENED}.{}@example.test>",
        TODAY[OPENED].messages - 1
    ));
    let Some(id) = messages
        .ids_by_rfc_message_id(account, &newest)
        .await
        .expect("a lookup")
        .first()
        .copied()
    else {
        return;
    };
    let (fixture, from, subject, to) = if screen == "27" {
        (
            "html-newsletter-own-page",
            EmailAddress::new(Some("Field Notes Weekly"), "news@example.com"),
            "Issue 48: The quiet season",
            EmailAddress::new(Some("You"), "you@example.com"),
        )
    } else if newsletter {
        (
            "html-newsletter-many-tables",
            EmailAddress::new(Some("Example Tools"), "news@tools.example.com"),
            "Release notes: a faster deploy",
            EmailAddress::new(Some("You"), "you@example.com"),
        )
    } else {
        (
            "html-work-black-text",
            EmailAddress::new(Some("Dana Whitfield"), "facilities@example.com"),
            "Building access changes from Monday",
            EmailAddress::new(None::<String>, "all-staff@example.com"),
        )
    };
    if let Some(mut message) = messages.get(id).await.expect("a read") {
        message.from = vec![from];
        message.subject = Some(subject.to_owned());
        message.to = vec![to];
        message.cc = Vec::new();
        messages.update(&mut message).await.expect("its headers");
    }
    let body = postio_model::mime::parse(postio_model::test_corpus::load(fixture).bytes()).body;
    messages
        .set_body(
            id,
            &postio_storage::repository::StoredBody {
                text: body.text,
                html: body.html,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .expect("a body");
    let markers = MarkerRepository::new(&connection);
    if newsletter {
        markers.dismiss(id, None).await.expect("no card");
    } else {
        markers
            .replace(&marker(
                id,
                &Ask::Question("Please confirm by Friday that your team has seen this."),
                today(),
            ))
            .await
            .expect("its card");
    }
}

/// The row screen 06 replies to: "Harbor API draft v3".
pub const HARBOR: u32 = 3;

/// The demo store: the storage seed, and today's inbox on top of it.
pub async fn demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let report = postio_storage::seed::seed_small(&database, 1).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed files an inbox")
        .id;
    let today = today();
    let connection = database.connect().await.expect("a connection");
    synced(&connection, inbox, today).await;
    let mut labels: HashMap<&str, postio_model::LabelId> = HashMap::new();
    for (index, row) in TODAY.iter().enumerate() {
        let at = today - chrono::Duration::minutes(row.minutes);
        let mut previous: Option<RfcMessageId> = None;
        let mut last = MessageId::UNASSIGNED;
        // The earlier messages of the conversation first, read, then the
        // one the row shows.
        for step in 0..row.messages {
            let later = (row.messages - 1 - step) as i64;
            let newest = later == 0;
            let when = at - chrono::Duration::minutes(40 * later);
            let mut message = message(report.account.id, inbox, row, when, newest);
            if newest && index == OPENED {
                message.attachments = opened_parts();
                // Who it went to and who was copied, as screen 04 draws them.
                message.to = vec![
                    EmailAddress::new(Some("You"), "you@example.com"),
                    EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net"),
                    EmailAddress::new(Some("Grace Oyelaran"), "grace@example.org"),
                ];
                message.cc = vec![EmailAddress::new(
                    Some("Harbor API"),
                    "harbor-api@example.org",
                )];
            }
            let id = RfcMessageId::new(format!("<demo.{index}.{step}@example.test>"));
            message.rfc_message_id = Some(id.clone());
            if let Some(parent) = previous.replace(id) {
                message.in_reply_to = Some(parent.clone());
                message.references = vec![parent];
            }
            MessageRepository::new(&connection)
                .create(&mut message)
                .await
                .expect("a message");
            // Sync records every address it sees; the bar reads names from
            // them (screen 07).
            ContactRepository::new(&connection)
                .record_message(&message)
                .await
                .expect("its correspondents");
            ThreadingRepository::new(&connection, report.account.id)
                .thread(&message)
                .await
                .expect("threaded");
            last = message.id;
        }
        let body = if index == OPENED {
            OPENED_BODY.to_owned()
        } else {
            format!("{}\n", row.preview)
        };
        MessageRepository::new(&connection)
            .set_body(
                last,
                &postio_storage::repository::StoredBody {
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
        let repository = LabelRepository::new(&connection);
        for name in row.labels {
            let label = match labels.get(name) {
                Some(label) => *label,
                None => {
                    let mut label = Label::new(report.account.id, *name);
                    repository.create(&mut label).await.expect("a label");
                    labels.insert(name, label.id);
                    label.id
                }
            };
            repository.attach(last, label).await.expect("labelled");
        }
        if let Some(ask) = &row.ask {
            MarkerRepository::new(&connection)
                .insert(&marker(last, ask, today))
                .await
                .expect("a marker");
        }
    }
    // Screens 07 and 08: invoices from Marisol filed in three places, and
    // a folder of receipts.
    let receipts = postio_storage::test_support::mailbox(&connection, &report.account, "Receipts")
        .await
        .id;
    let archive = report
        .mailbox(MailboxRole::Archive)
        .map_or(inbox, |mailbox| mailbox.id);
    // The 19th of last month, whatever today is: "last month" is a
    // calendar month.
    let last_month = {
        use chrono::Datelike as _;
        let first = today.date_naive().with_day(1).unwrap_or(today.date_naive());
        let first = first
            .checked_sub_months(chrono::Months::new(1))
            .unwrap_or(first);
        let nineteenth = first.with_day(19).unwrap_or(first);
        today - (today.date_naive() - nineteenth)
    };
    for (mailbox, subject, preview, days) in [
        (
            inbox,
            "Invoice 2026-08, Atlas contractor hours",
            "Attached is the invoice for August.",
            0,
        ),
        (
            receipts,
            "Re: Invoice 2026-08 (corrected)",
            "Fixed the PO number, same total.",
            5,
        ),
        (
            archive,
            "Invoice for July (late)",
            "Sorry for the delay. This one covers three weeks.",
            11,
        ),
    ] {
        filed(
            &connection,
            report.account.id,
            mailbox,
            ("Marisol Quint", "marisol@example.com"),
            subject,
            preview,
            last_month + chrono::Duration::days(days),
        )
        .await;
    }
    for (subject, days) in [
        ("Your coffee order", 2),
        ("Bookshop receipt", 9),
        ("Train ticket", 16),
    ] {
        filed(
            &connection,
            report.account.id,
            receipts,
            ("Receipts", "receipts@shop.example"),
            subject,
            "Thank you for your order.",
            today - chrono::Duration::days(days),
        )
        .await;
    }
    // Screen 01's digest row: this week's newsletters, held for
    // "Newsletters" and delivered at 16:00 today.
    let digests = postio_storage::repository::DigestRepository::new(&connection);
    let mut newsletter_ids = Vec::with_capacity(NEWSLETTERS.len());
    for (index, (from, subject, minutes)) in NEWSLETTERS.iter().enumerate() {
        let at = today - chrono::Duration::minutes(*minutes);
        let mut message = Message::new(report.account.id, inbox, at);
        message.date = Some(at);
        message.from = vec![EmailAddress::new(Some(from.0), from.1)];
        message.subject = Some((*subject).to_owned());
        message.preview = Some("This week's issue.".to_owned());
        message.flags = [Flag::Seen].into_iter().collect();
        message.rfc_message_id = Some(RfcMessageId::new(format!(
            "<newsletter.{index}@example.test>"
        )));
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a newsletter");
        digests.hold(id, "Newsletters", at).await.expect("held");
        newsletter_ids.push(id);
    }
    let due = today - chrono::Duration::minutes(9);
    let delivery = digests
        .deliver("Newsletters", due, due)
        .await
        .expect("delivered")
        .expect("a delivery");
    // Screens 22 and 23: a summary of two of the newsletters, citing the
    // rate decision and the CRDT comparison, each with a body the cited
    // passage is found in verbatim.
    let rate_body = "Good morning.\n\n\
        The committee held the rate at four percent for a third month.\n\n\
        The Ledger\n";
    let crdt_body = "This week: three CRDT libraries compared on the same workload, \
        and a long read on garbage collection for long-lived documents.\n\n\
        1. The comparison\n\nWe replayed the same editing trace, 260,000 operations \
        recorded from a real shared document.\n";
    for (id, body) in [
        (newsletter_ids[0], rate_body),
        (newsletter_ids[3], crdt_body),
    ] {
        MessageRepository::new(&connection)
            .set_body(
                id,
                &postio_storage::repository::StoredBody {
                    text: Some(body.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                postio_model::BodyState::Full,
            )
            .await
            .expect("a body");
    }
    let statement = |topic: &str, text: &str, number: u32, message, excerpt: &str| {
        postio_model::summary::SummaryStatement {
            topic: topic.to_owned(),
            text: text.to_owned(),
            reference: postio_model::summary::SummaryReference {
                number,
                message,
                excerpt: excerpt.to_owned(),
            },
        }
    };
    let summary = postio_model::summary::DigestSummary {
        statements: vec![
            statement(
                "Rates",
                "The committee held the rate at four percent for a third month.",
                1,
                newsletter_ids[0],
                "held the rate at four percent",
            ),
            statement(
                "Engineering reading",
                "Crate Notes compares three CRDT libraries on the same workload.",
                2,
                newsletter_ids[3],
                "three CRDT libraries compared on the same workload",
            ),
        ],
        messages: NEWSLETTERS.len() as u32,
        senders: 5,
    };
    postio_storage::repository::DigestRepository::new(&connection)
        .set_summary(
            delivery,
            &serde_json::to_string(&summary).expect("it serialises"),
            due,
        )
        .await
        .expect("a summary");
    // Screen 21: what filing archived today, each with its reason.
    for (index, (from, subject, preview, reason, source, minutes)) in FILTERED.iter().enumerate() {
        let at = today - chrono::Duration::minutes(*minutes);
        let mut message = Message::new(report.account.id, archive, at);
        message.date = Some(at);
        message.from = vec![EmailAddress::new(Some(from.0), from.1)];
        message.subject = Some((*subject).to_owned());
        message.preview = Some((*preview).to_owned());
        message.flags = [Flag::Seen].into_iter().collect();
        message.rfc_message_id = Some(RfcMessageId::new(format!(
            "<filtered.{index}@example.test>"
        )));
        let id = MessageRepository::new(&connection)
            .create(&mut message)
            .await
            .expect("a filtered message");
        postio_storage::repository::FilterDecisionRepository::new(&connection)
            .record(&postio_storage::repository::FilterDecision {
                message: id,
                reason: postio_storage::repository::FilterReason::from_name(reason)
                    .expect("a reason"),
                source: source.map(str::to_owned),
                layer: postio_storage::repository::FilterLayer::Header,
                decided_at: at,
            })
            .await
            .expect("a decision");
    }
    // Screen 14: mail was last moved to Receipts.
    postio_storage::repository::SettingsRepository::new(&connection)
        .note_move(receipts)
        .await
        .expect("a recent move");
    // The command bar searches this machine's index, as sync fills it.
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the search index");
    drop(connection);
    (database, report.account.id)
}

/// File one message from `from` about `subject` into `mailbox` at `at`.
pub async fn filed(
    connection: &postio_storage::Connection,
    account: AccountId,
    mailbox: MailboxId,
    from: (&str, &str),
    subject: &str,
    preview: &str,
    at: DateTime<Utc>,
) {
    let mut message = Message::new(account, mailbox, at);
    message.date = Some(at);
    message.from = vec![EmailAddress::new(Some(from.0), from.1)];
    message.subject = Some(subject.to_owned());
    message.preview = Some(preview.to_owned());
    message.flags = [Flag::Seen].into_iter().collect();
    message.rfc_message_id = Some(RfcMessageId::new(format!(
        "<filed.{}@example.test>",
        subject.to_lowercase().replace(' ', ".")
    )));
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("a filed message");
    ContactRepository::new(connection)
        .record_message(&message)
        .await
        .expect("its correspondents");
    ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("threaded");
}

pub fn message(
    account: AccountId,
    inbox: MailboxId,
    row: &Row,
    at: DateTime<Utc>,
    newest: bool,
) -> Message {
    let mut message = Message::new(account, inbox, at);
    message.date = Some(at);
    if newest {
        message.from = vec![EmailAddress::new(Some(row.name), row.address)];
        message.preview = Some(row.preview.to_owned());
    } else {
        message.from = vec![EmailAddress::new(Some("You"), "you@example.com")];
        message.preview = Some("Earlier in the conversation.".to_owned());
    }
    message.to = vec![EmailAddress::new(Some("You"), "you@example.com")];
    message.subject = Some(row.subject.to_owned());
    let mut flags = FlagSet::new();
    if !(newest && row.unread) {
        flags.insert(Flag::Seen);
    }
    message.flags = flags;
    if newest && row.attachment {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 48_000);
        attachment.filename = Some("attached.pdf".to_owned());
        message.attachments.push(attachment);
    }
    message
}

pub fn marker(message: MessageId, ask: &Ask, today: DateTime<Utc>) -> Marker {
    let midnight = today - chrono::Duration::minutes(16 * 60 + 9);
    let blank = Marker {
        message,
        kind: MarkerKind::Question,
        source: MarkerSource::Detector,
        span: None,
        excerpt: None,
        starts_at: None,
        ends_at: None,
        due_at: None,
        invite: None,
        invite_state: None,
        answer: None,
        dismissed_at: None,
    };
    match ask {
        Ask::Invite {
            days,
            hour,
            minutes,
        } => {
            let starts = midnight + chrono::Duration::days(*days) + chrono::Duration::hours(*hour);
            Marker {
                kind: MarkerKind::Invite,
                source: MarkerSource::Calendar,
                starts_at: Some(starts),
                ends_at: Some(starts + chrono::Duration::minutes(*minutes)),
                ..blank
            }
        }
        Ask::Question(sentence) => Marker {
            span: Some((0, sentence.len() as u32)),
            excerpt: Some((*sentence).to_owned()),
            ..blank
        },
        Ask::Todo { days, sentence } => Marker {
            kind: MarkerKind::Todo,
            span: Some((0, sentence.len() as u32)),
            excerpt: Some((*sentence).to_owned()),
            due_at: Some(midnight + chrono::Duration::days(*days) + chrono::Duration::hours(17)),
            ..blank
        },
    }
}

/// The runtime the demo's reads are driven on.
///
/// `block_on` polls the future on *this* thread, where GTK lives, and
/// `multi_thread` because the store's reads from a synchronous callback reach
/// for `block_in_place`.
pub fn on_runtime<T>(future: impl std::future::Future<Output = T>) -> T {
    use std::sync::OnceLock;
    static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
                .expect("a runtime for the demo")
        })
        .block_on(future)
}

/// A window adopted over a started host, and what a tool needs to drive it.
pub struct Started {
    /// Focus's window, presented and adopted.
    pub window: FocusWindow,
    /// The session over the host: stop it before the window goes.
    pub session: Session,
    /// Where the engine's events enter: a tool says what sync would have.
    pub sink: postio_core::bridge::EventSink,
    /// The account the store was seeded with.
    pub account: AccountId,
    /// The blob directory the host reads, kept until the demo ends.
    _blobs: tempfile::TempDir,
}

impl Started {
    /// Take the window down, stopping its session first.
    pub fn finish(self) {
        self.window.destroy();
        self.session.stop();
    }
}

/// Start the host over `database`, present a Focus window of `size` and
/// adopt it, as the application starts: the path `shot` and the storyboard
/// runner share.
///
/// `config` is the TOML the window runs under, written to `config_path`
/// first: Settings shows the file it writes, and writes to it.
pub fn start(
    database: Store,
    account: AccountId,
    config: &str,
    config_path: &std::path::Path,
    size: (i32, i32),
) -> Result<Started, String> {
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("no config folder: {error}"))?;
    }
    std::fs::write(config_path, config).map_err(|error| format!("no config: {error}"))?;
    let config = postio_config::Config::from_toml_str(config)
        .map_err(|error| format!("the demo's config: {error}"))?;
    let blobs_dir = tempfile::tempdir().map_err(|error| format!("no scratch: {error}"))?;
    let blobs = BlobStore::open(
        blobs_dir.path().to_path_buf(),
        &postio_storage::test_support::blob_keys(),
    )
    .map_err(|error| format!("no blob store: {error}"))?;
    let sink = std::rc::Rc::new(std::cell::RefCell::new(None));
    let host = postio_host::Host::start(database, blobs, {
        let sink = std::rc::Rc::clone(&sink);
        move |wiring| {
            sink.replace(Some(wiring.events.clone()));
            wiring
        }
    })
    .map_err(|error| format!("the host did not start: {error}"))?;
    let sink = sink.take().ok_or("the host kept its events to itself")?;
    let window = FocusWindow::new(None);
    window.set_default_size(size.0, size.1);
    window.present();
    let session = crate::startup::adopt_at(&window, host, &config, Some(config_path));
    Ok(Started {
        window,
        session,
        sink,
        account,
        _blobs: blobs_dir,
    })
}
