//! Focus's demo stores, shared by every app that photographs or drives Focus; toolkit-free so apps
//! share one seed. Test-support code: an app reaches it only behind its own `demo` feature.
//! Every name is invented, every address is on a reserved domain, and nothing touches the network.

mod files;
mod search;
mod threads;

pub use files::{SearchFile, search_files};
pub use search::{search_demo, store_search_blobs};
pub use threads::{Arrival, deliver_harbor_reply, store_threads_blobs, threads_demo};

/// The bases decide what mail the store holds; the rest add one thing to the small inbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    Small,
    Empty,
    LongNewsletter,
    BareRecipients,
    LongThread,
    ThirtyThreads,
    TwoAccounts,
    Outbox,
    DraftLeftOver,
    Backfilling,
    Search,
    Threads,
}

impl Seed {
    pub const ALL: [Seed; 12] = [
        Seed::Small,
        Seed::Empty,
        Seed::LongNewsletter,
        Seed::BareRecipients,
        Seed::LongThread,
        Seed::ThirtyThreads,
        Seed::TwoAccounts,
        Seed::Outbox,
        Seed::DraftLeftOver,
        Seed::Backfilling,
        Seed::Search,
        Seed::Threads,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Seed::Small => "small",
            Seed::Empty => "empty",
            Seed::LongNewsletter => "long-newsletter",
            Seed::BareRecipients => "bare-recipients",
            Seed::LongThread => "long-thread",
            Seed::ThirtyThreads => "thirty-threads",
            Seed::TwoAccounts => "two-accounts",
            Seed::Outbox => "outbox",
            Seed::DraftLeftOver => "draft-left-over",
            Seed::Backfilling => "backfilling",
            Seed::Search => "search",
            Seed::Threads => "threads",
        }
    }

    pub fn from_id(id: &str) -> Option<Seed> {
        Seed::ALL.into_iter().find(|seed| seed.id() == id)
    }
}

/// The store half only: a backfill in flight is an event, which `postio-gtk`'s `demo::storyboard`
/// says once the host is up.
pub async fn seeded(seed: Seed) -> (Store, AccountId) {
    match seed {
        Seed::Empty => empty_demo().await,
        Seed::Search => search_demo().await,
        Seed::Threads => threads_demo().await,
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
        | Seed::BareRecipients
        | Seed::TwoAccounts
        | Seed::Outbox
        | Seed::DraftLeftOver
        | Seed::Backfilling => {
            let (database, account) = demo().await;
            match seed {
                Seed::LongNewsletter => treatment_demo(&database, account, "30").await,
                Seed::BareRecipients => {
                    let connection = database.connect().await.expect("a connection");
                    let messages = MessageRepository::new(&connection);
                    let newest = RfcMessageId::new(format!(
                        "<demo.{OPENED}.{}@example.test>",
                        TODAY[OPENED].messages - 1
                    ));
                    for id in messages
                        .ids_by_rfc_message_id(account, &newest)
                        .await
                        .expect("the opened message")
                    {
                        let mut message =
                            messages.get(id).await.expect("a message").expect("saved");
                        message.to = [
                            "engineering.accounts.team@example.com",
                            "operations.accounts.team@example.org",
                            "procurement.accounts.team@example.net",
                        ]
                        .into_iter()
                        .map(|address| EmailAddress::new(None::<String>, address))
                        .collect();
                        messages.update(&mut message).await.expect("addressed");
                    }
                }
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
use postio_model::listing::MarkerKind;
use postio_model::{
    AccountId, Attachment, EmailAddress, Flag, FlagSet, Label, MailboxId, MailboxRole, Message,
    MessageId, RfcMessageId,
};
use postio_storage::Store;
use postio_storage::repository::{
    ContactRepository, LabelRepository, MailboxRepository, Marker, MarkerRepository, MarkerSource,
    MessageRepository, ThreadingRepository,
};

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

pub enum Ask {
    Invite { days: i64, hour: i64, minutes: i64 },
    Question(&'static str),
    Todo { days: i64, sentence: &'static str },
}

pub struct Row {
    name: &'static str,
    address: &'static str,
    subject: &'static str,
    preview: &'static str,
    minutes: i64,
    unread: bool,
    labels: &'static [&'static str],
    attachment: bool,
    messages: usize,
    ask: Option<Ask>,
}

pub const NEW_SENDER: &str = "caspian@example.com";

pub const TODAY: &[Row] = &[
    Row {
        name: "Hollis Varga",
        address: "hollis@example.com",
        subject: "Invitation: Harbor design review",
        preview: "Friday 10:00\u{2013}10:45, Room 3B. Agenda: navigation, empty states, the export flow.",
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
            days: 1,
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
            days: 6,
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
            days: 5,
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

/// The rule comes due on the demo's own day, 16:00, so the delivery the seed makes and the row's
/// "came due" agree.
pub fn config() -> String {
    let day = today()
        .with_timezone(&Local)
        .format("%A")
        .to_string()
        .to_lowercase();
    format!(
        "[focus]\nfiltering = true\n\n[[focus.digests]]\nname = \"Newsletters\"\n\
match = [\"from:news@ledger.example\", \"from:hello@soil.example\", \"from:notes@crate.example\", \"from:tides@tide.example\", \"from:digest@harbor.example\"]\ncadence = \"weekly\"\nday = \"{day}\"\nat = \"16:00\"\n\n\
[saved_searches.waiting]\nquery = \"from:juno\"\npinned = true\norder = 1\nname = \"Waiting on reply\"\n\n\
[saved_searches.atlas]\nquery = \"subject:atlas\"\npinned = true\norder = 2\nname = \"Atlas\"\n\n\
[saved_searches.receipts]\nquery = \"in:Receipts\"\npinned = true\norder = 3\nname = \"Receipts this month\"\n\n\
[saved_searches.school]\nquery = \"from:northfield\"\npinned = true\norder = 4\nname = \"From school\"\n"
    )
}

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

/// Hands down the addresses the store's accounts send as, so they are not counted as people.
pub(crate) async fn record_correspondents(
    connection: &postio_storage::Connection,
    message: &postio_model::Message,
) -> postio_storage::Result<usize> {
    let own = postio_storage::repository::IdentityRepository::new(connection)
        .own_addresses()
        .await?;
    ContactRepository::new(connection)
        .record_message(message, &own)
        .await
}

pub async fn compose_demo(database: &Store, account: AccountId) {
    let connection = database.connect().await.expect("a connection");
    let contacts = postio_storage::repository::ContactRepository::new(&connection);
    for (name, address) in [
        ("Grace Oyelaran", "grace@example.org"),
        ("Graham Ellis", "graham@example.net"),
        ("Ada Moreno", "ada@example.com"),
    ] {
        // The demo's mail may already have made them someone; a second
        // person for the same address is refused.
        if contacts
            .by_address(address)
            .await
            .expect("a lookup")
            .is_some()
        {
            continue;
        }
        contacts
            .create(Some(name), &[EmailAddress::new(Some(name), address)])
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

const EVENTS: [(&str, i64, i64, i64, &str); 6] = [
    ("Standup", 0, 9 * 60 + 30, 15, "Room 2"),
    ("Design review", 0, 14 * 60, 60, "Room 4.12"),
    ("Dentist", 0, 17 * 60, 30, "Harbour Street clinic"),
    ("Standup", 1, 9 * 60 + 30, 15, "Room 2"),
    ("Lunch with Ines", 1, 12 * 60 + 30, 60, "Corner café"),
    ("Quarterly planning", 2, 10 * 60, 120, "Room 4.12"),
];

async fn calendar_demo(connection: &postio_storage::Connection, today: DateTime<Utc>) {
    use postio_storage::repository::{CalendarRepository, StoredEvent, StoredOccurrence};
    let calendars = CalendarRepository::new(connection);
    let local = calendars.local().await.expect("On this computer").id;
    let midnight = today.with_timezone(&Local).date_naive();
    for (n, (title, day, minute, length, place)) in EVENTS.into_iter().enumerate() {
        let start = Local
            .from_local_datetime(
                &(midnight + chrono::Duration::days(day)).and_time(chrono::NaiveTime::MIN),
            )
            .earliest()
            .map(|at| at.to_utc())
            .unwrap_or(today)
            + chrono::Duration::minutes(minute);
        let end = start + chrono::Duration::minutes(length);
        let stamp = |at: DateTime<Utc>| at.format("%Y%m%dT%H%M%SZ").to_string();
        let uid = format!("demo-event-{n}@example.test");
        let object = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//Postio demo//EN\r\nBEGIN:VEVENT\r\n\
             UID:{uid}\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:{title}\r\n\
             LOCATION:{place}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
            stamp(start),
            stamp(start),
            stamp(end)
        );
        let event = calendars
            .put_event(&StoredEvent {
                calendar: local,
                uid,
                href: None,
                etag: None,
                object: object.into_bytes(),
                title: title.to_owned(),
                location: place.to_owned(),
                starts_at: start,
                recurs: false,
                status: postio_model::calendar::EventStatus::Confirmed,
                my_partstat: None,
                source_message: None,
            })
            .await
            .expect("a demo event");
        calendars
            .set_occurrences(
                event,
                &[StoredOccurrence {
                    start,
                    end,
                    all_day: false,
                    floating: false,
                    recurrence_id: None,
                }],
            )
            .await
            .expect("its occurrence");
    }
}

pub fn today() -> DateTime<Utc> {
    postio_model::clock::now()
        .date_naive()
        .and_hms_opt(16, 9, 0)
        .and_then(|at| Local.from_local_datetime(&at).single())
        .map(|at| at.with_timezone(&Utc))
        .unwrap_or_else(|| postio_model::clock::now().to_utc())
}

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

const HELD: usize = 0;

const ARRIVAL: Row = Row {
    name: "Ines Marlow",
    address: "ines@example.com",
    subject: "Lunch on Thursday?",
    preview: "Are you free at noon? The new place by the harbor has a table.",
    minutes: 0,
    unread: true,
    labels: &[],
    attachment: false,
    messages: 1,
    ask: None,
};

fn text_of(error: impl std::fmt::Display) -> String {
    error.to_string()
}

async fn held_message(
    connection: &postio_storage::Connection,
    account: AccountId,
) -> Result<MessageId, String> {
    let rfc = RfcMessageId::new(format!("<demo.{HELD}.0@example.test>"));
    MessageRepository::new(connection)
        .ids_by_rfc_message_id(account, &rfc)
        .await
        .map_err(text_of)?
        .into_iter()
        .next()
        .ok_or_else(|| "the seed has no message to hold a body back from".to_owned())
}

pub async fn hold_back_body(store: &Store, account: AccountId) -> Result<(), String> {
    let connection = store.connect().await.map_err(text_of)?;
    let id = held_message(&connection, account).await?;
    MessageRepository::new(&connection)
        .set_body(
            id,
            &postio_storage::repository::StoredBody {
                text: None,
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::HeadersOnly,
        )
        .await
        .map_err(text_of)?;
    Ok(())
}

pub async fn deliver_held_body(store: &Store, account: AccountId) -> Result<MessageId, String> {
    let connection = store.connect().await.map_err(text_of)?;
    let id = held_message(&connection, account).await?;
    MessageRepository::new(&connection)
        .set_body(
            id,
            &postio_storage::repository::StoredBody {
                text: Some(format!("{}\n", TODAY[HELD].preview)),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .map_err(text_of)?;
    Ok(id)
}

pub async fn deliver_new_mail(
    store: &Store,
    account: AccountId,
) -> Result<(MailboxId, MessageId), String> {
    let connection = store.connect().await.map_err(text_of)?;
    let inbox = MailboxRepository::new(&connection)
        .by_role(account, MailboxRole::Inbox)
        .await
        .map_err(text_of)?
        .ok_or("the account has no inbox")?
        .id;
    let mut message = message(account, inbox, &ARRIVAL, today(), true);
    message.rfc_message_id = Some(RfcMessageId::new("<demo.arrival@example.test>"));
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .map_err(text_of)?;
    record_correspondents(&connection, &message)
        .await
        .map_err(text_of)?;
    ThreadingRepository::new(&connection, account)
        .thread(&message)
        .await
        .map_err(text_of)?;
    MessageRepository::new(&connection)
        .set_body(
            message.id,
            &postio_storage::repository::StoredBody {
                text: Some(format!("{}\n", ARRIVAL.preview)),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            postio_model::BodyState::Full,
        )
        .await
        .map_err(text_of)?;
    MailboxRepository::new(&connection)
        .recount(inbox)
        .await
        .map_err(text_of)?;
    Ok((inbox, message.id))
}

pub const OPENED: usize = 3;

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
        part.part_id = Some((index + 2).to_string());
        part
    })
    .collect()
}

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
        message.preview = Some(format!(
            "{subject}. Read the release notes in your browser."
        ));
        if newsletter {
            message.attachments.clear();
        }
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
        // A newsletter carries no marker: `None` would take a dismissal
        // back, so the day says it is gone.
        markers.dismiss(id, Some(today())).await.expect("no card");
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

pub const HARBOR: u32 = 3;

pub async fn demo() -> (Store, AccountId) {
    let database = postio_storage::test_support::memory().await;
    let report = postio_storage::seed::seed_small_downloaded(&database, 1).await;
    let inbox = report
        .mailbox(MailboxRole::Inbox)
        .expect("the seed files an inbox")
        .id;
    let today = today();
    let connection = database.connect().await.expect("a connection");
    synced(&connection, inbox, today).await;
    calendar_demo(&connection, today).await;
    let mut labels: HashMap<&str, postio_model::LabelId> = HashMap::new();
    for (index, row) in TODAY.iter().enumerate() {
        let at = today - chrono::Duration::minutes(row.minutes);
        let mut previous: Option<RfcMessageId> = None;
        let mut last = MessageId::UNASSIGNED;
        for step in 0..row.messages {
            let later = (row.messages - 1 - step) as i64;
            let newest = later == 0;
            let when = at - chrono::Duration::minutes(40 * later);
            let mut message = message(report.account.id, inbox, row, when, newest);
            if newest && index == OPENED {
                message.attachments = opened_parts();
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
            if newest && row.address == NEW_SENDER {
                message.cc = vec![EmailAddress::new(Some("Ben Adeyemi"), "ben@example.net")];
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
            record_correspondents(&connection, &message)
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
        } else if row.address == NEW_SENDER {
            format!(
                "{}\n\nBest,\nCaspian Holt\nPartnerships \u{b7} Harbor Data Co.\n+1 555 0142\nharbordata.example\n",
                row.preview
            )
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
    let receipts = postio_storage::test_support::mailbox(&connection, &report.account, "Receipts")
        .await
        .id;
    let archive = report
        .mailbox(MailboxRole::Archive)
        .map_or(inbox, |mailbox| mailbox.id);
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
    waiting_on(
        &connection,
        &report.account,
        archive,
        ("Marisol Quint", "marisol@example.com"),
        "Q4 headcount numbers",
        today - chrono::Duration::days(2),
    )
    .await;
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
        record_correspondents(&connection, &message)
            .await
            .expect("its correspondents");
        digests.hold(id, "Newsletters", at).await.expect("held");
        newsletter_ids.push(id);
    }
    // One transaction for the whole run: every window of the every-command pass seeds a store.
    postio_storage::sql::batch(&connection, "BEGIN IMMEDIATE")
        .await
        .expect("begin");
    let mut sent = std::collections::HashSet::new();
    for ((name, address), _, _) in NEWSLETTERS {
        if !sent.insert(*address) {
            continue;
        }
        for week in 1..=3 {
            filed(
                &connection,
                report.account.id,
                archive,
                (name, address),
                &format!("Issue {}", 50 - week),
                "An earlier issue.",
                today - chrono::Duration::days(7 * week),
            )
            .await;
        }
    }
    postio_storage::sql::batch(&connection, "COMMIT")
        .await
        .expect("commit");
    let due = today - chrono::Duration::minutes(9);
    let delivery = digests
        .deliver("Newsletters", due, due)
        .await
        .expect("delivered")
        .expect("a delivery");
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
    postio_storage::repository::SettingsRepository::new(&connection)
        .note_move(receipts)
        .await
        .expect("a recent move");
    // Before the search index exists: its triggers index every message
    // this makes, and `ensure_schema` backfills the rest in one pass.
    people(&connection).await;
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("the search index");
    drop(connection);
    (database, report.account.id)
}

pub const IMPORT_VCF: &str = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:demo-grace\r\nFN:Grace Oyelaran\r\nEMAIL:grace@example.org\r\nORG:Example Labs\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:demo-ben\r\nFN:Ben Adeyemi\r\nEMAIL:ben@adeyemi.example\r\nTEL:+1 555 0188\r\nEND:VCARD\r\n\
BEGIN:VCARD\r\nVERSION:4.0\r\nUID:urn:uuid:demo-tove\r\nFN:Tove Larsson\r\nEMAIL:tove.larsson@example.net\r\nORG:Larsson Studio\r\nEND:VCARD\r\n";

/// Mail from it is in the demo; none of it is a person a screenshot should show.
const ABUSE_DOMAIN: &str = "abuse.example.com";

/// Their mail is in the demo; they are not among its people.
const HOSTILE_SENDERS: &[&str] = &[
    "mallory@sender.invalid",
    "billing@sender.invalid",
    "broken.from@example.com",
];

async fn sort_by_kind(connection: &postio_storage::Connection) {
    let contacts = ContactRepository::new(connection);
    let rules = postio_classify::kind::KindRules::shipped();
    let version = postio_classify::kind::KIND_VERSION;
    loop {
        let people = contacts
            .pending_kinds(version, 500)
            .await
            .expect("people to sort");
        for person in &people {
            let kind = match contacts.kind_signals(*person).await.expect("signals") {
                Some(signals) => postio_classify::kind::detect(&signals, rules).kind,
                None => postio_model::ContactKind::Person,
            };
            contacts
                .set_detected_kind(*person, kind, version)
                .await
                .expect("a detected kind");
        }
        if people.len() < 500 {
            break;
        }
    }
}

async fn people(connection: &postio_storage::Connection) {
    postio_storage::sql::batch(connection, "BEGIN IMMEDIATE")
        .await
        .expect("begin");
    ContactRepository::new(connection)
        .catch_up_batch(None, 100_000)
        .await
        .expect("a person for every address");
    // The corpus's attack senders stay in the corpus: nobody has a contact named by a header
    // injection.
    for hostile in HOSTILE_SENDERS {
        postio_storage::sql::execute(
            connection,
            "UPDATE contacts SET state = 'deleted'
              WHERE id IN (SELECT contact_id FROM addresses
                            WHERE address_normalized = ?1 OR address_normalized LIKE ?2)",
            postio_storage::sql::bind![*hostile, format!("%@{ABUSE_DOMAIN}")],
        )
        .await
        .expect("a hostile sender put away");
    }
    sort_by_kind(connection).await;
    let mut kinds: Vec<(&str, String)> = NEWSLETTERS
        .iter()
        .map(|((_, address), _, _)| ("newsletter", (*address).to_owned()))
        .collect();
    for (kind, address) in [
        ("list", "announce@lists.example.org"),
        ("list", "all-staff@example.com"),
        ("list", "finance@example.org"),
        ("automated", "reports@builds.example.com"),
        ("automated", "receipts@shop.example"),
        ("automated", "no-reply@city.example"),
        // Detection calls a role mailbox a company only where the user knows two of its people; the
        // demo says what a fuller mailbox would.
        ("company", "office@northfield.example"),
    ] {
        kinds.push((kind, address.to_owned()));
    }
    let mut seen = std::collections::HashSet::new();
    kinds.retain(|(_, address)| seen.insert(address.clone()));
    for (kind, address) in &kinds {
        postio_storage::sql::execute(
            connection,
            "UPDATE contacts SET kind = ?2, kind_source = 'user'
              WHERE id = (SELECT contact_id FROM addresses WHERE address_normalized = ?1)",
            postio_storage::sql::bind![address.as_str(), *kind],
        )
        .await
        .expect("a kind");
        if *kind == "newsletter" {
            postio_storage::sql::execute(
                connection,
                "INSERT INTO contact_settings
                   (contact_id, arrival, digest_name, digest_cadence, digest_day, digest_at)
                 SELECT contact_id, 'digest', 'Newsletters', 'weekly', 'Saturday', '16:00'
                   FROM addresses WHERE address_normalized = ?1",
                postio_storage::sql::bind![address.as_str()],
            )
            .await
            .expect("a digest setting");
        }
    }
    {
        let contacts = ContactRepository::new(connection);
        for (name, address, seen, saved) in [
            ("Ada Moreno", "ada.moreno@northwind.example", 412, true),
            ("A. Moreno", "a.moreno@example.net", 14, false),
            ("Ada M.", "ada.m@example.org", 6, false),
        ] {
            let person = contacts
                .create(
                    Some(name),
                    &[postio_model::EmailAddress::new(None::<String>, address)],
                )
                .await
                .expect("a record");
            postio_storage::sql::execute(
                connection,
                "INSERT OR REPLACE INTO contact_sightings
                     (address_id, account_id, times_seen, last_seen_at, last_name)
                 SELECT a.id, (SELECT id FROM accounts LIMIT 1), ?2, ?3, ?4
                   FROM addresses a WHERE a.contact_id = ?1",
                postio_storage::sql::bind![person.get(), seen, 1_790_000_000_000_i64, name],
            )
            .await
            .expect("their sightings");
            for back in 0..6_i64 {
                let share = seen / 6 + if back == 0 { seen % 6 } else { 0 };
                postio_storage::sql::execute(
                    connection,
                    "INSERT OR REPLACE INTO address_months (address_id, month, n)
                     SELECT a.id, ?2 - ?3, ?4 FROM addresses a WHERE a.contact_id = ?1",
                    postio_storage::sql::bind![
                        person.get(),
                        postio_model::month_key(
                            chrono::DateTime::from_timestamp_millis(1_790_000_000_000)
                                .expect("a date")
                        ),
                        back,
                        share
                    ],
                )
                .await
                .expect("their months");
            }
            postio_storage::sql::execute(
                connection,
                "UPDATE contacts SET times_seen = ?2, last_seen_at = ?3, source = ?4,
                        saved_at = ?5, title = ?6, organization = ?7
                  WHERE id = ?1",
                postio_storage::sql::bind![
                    person.get(),
                    seen,
                    1_790_000_000_000_i64,
                    if saved { "user" } else { "mail" },
                    if saved { Some(1_i64) } else { None },
                    if saved { Some("Finance lead") } else { None },
                    if saved {
                        Some("Northwind Partners")
                    } else {
                        None
                    }
                ],
            )
            .await
            .expect("their counts");
        }
        let signature = "ada moreno|finance lead|northwind partners";
        for address in [
            "ada.moreno@northwind.example",
            "a.moreno@example.net",
            "ada.m@example.org",
        ] {
            postio_storage::sql::execute(
                connection,
                "INSERT INTO contact_evidence (contact_id, kind, value)
                 SELECT contact_id, 'signature', ?2 FROM addresses WHERE address_normalized = ?1",
                postio_storage::sql::bind![address, signature],
            )
            .await
            .expect("a signature noted");
        }
        postio_storage::sql::execute(
            connection,
            "INSERT INTO contact_evidence (contact_id, kind, value)
             SELECT contact_id, 'resent', 'ada.moreno@northwind.example'
               FROM addresses WHERE address_normalized = 'ada.m@example.org'",
            (),
        )
        .await
        .expect("a Resent-From noted");
        postio_storage::sql::batch(connection, "COMMIT")
            .await
            .expect("commit");
        for n in 0..2_i64 {
            postio_storage::sql::execute(
                connection,
                "INSERT INTO threads (account_id, subject) SELECT id, ?1 FROM accounts LIMIT 1",
                postio_storage::sql::bind![format!("Atlas actuals {n}")],
            )
            .await
            .expect("a thread");
            let thread = connection.last_insert_rowid();
            for (name, address, days) in [
                ("A. Moreno", "a.moreno@example.net", 40_i64),
                ("Ada Moreno", "ada.moreno@northwind.example", 20_i64),
            ] {
                let account: i64 =
                    postio_storage::sql::scalar(connection, "SELECT id FROM accounts LIMIT 1", ())
                        .await
                        .expect("the account");
                let archive: i64 = postio_storage::sql::scalar(
                    connection,
                    "SELECT id FROM mailboxes WHERE role = 'archive' LIMIT 1",
                    (),
                )
                .await
                .expect("the archive");
                let mut message = Message::new(
                    AccountId::new(account),
                    MailboxId::new(archive),
                    today() - chrono::Duration::days(days) - chrono::Duration::minutes(n),
                );
                message.from = vec![EmailAddress::new(Some(name), address)];
                message.to = vec![EmailAddress::new(Some("You"), "you@example.com")];
                message.subject = Some(format!("Atlas actuals {n}"));
                message.flags.insert(Flag::Seen);
                MessageRepository::new(connection)
                    .create(&mut message)
                    .await
                    .expect("a message in the archive");
                postio_storage::sql::execute(
                    connection,
                    "UPDATE messages SET thread_id = ?2 WHERE id = ?1",
                    postio_storage::sql::bind![message.id.get(), thread],
                )
                .await
                .expect("threaded");
            }
        }
    }
    for address in ["ben@example.net", "hollis@example.com"] {
        postio_storage::sql::execute(
            connection,
            "UPDATE contacts SET saved_at = 1, source = 'user', listed = 1
              WHERE id = (SELECT contact_id FROM addresses WHERE address_normalized = ?1)",
            postio_storage::sql::bind![address],
        )
        .await
        .expect("a saved person");
    }
    for statement in [
        "UPDATE contacts SET saved_at = 1, source = 'user',
                note = 'Prefers numbers in a sheet rather than in the email body.',
                organization = 'Northwind Partners', title = 'Finance lead'
          WHERE id = (SELECT contact_id FROM addresses WHERE address_normalized = 'marisol@example.com')",
        "INSERT INTO contact_settings (contact_id, notify, remind_days)
         SELECT contact_id, 1, 3 FROM addresses WHERE address_normalized = 'marisol@example.com'",
    ] {
        postio_storage::sql::execute(connection, statement, ())
            .await
            .expect("a person's details");
    }
}

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
    record_correspondents(connection, &message)
        .await
        .expect("its correspondents");
    ThreadingRepository::new(connection, account)
        .thread(&message)
        .await
        .expect("threaded");
}

async fn waiting_on(
    connection: &postio_storage::Connection,
    account: &postio_model::Account,
    mailbox: MailboxId,
    to: (&str, &str),
    subject: &str,
    at: DateTime<Utc>,
) {
    let mut message = Message::new(account.id, mailbox, at);
    message.date = Some(at);
    message.from = vec![account.address.clone()];
    message.to = vec![EmailAddress::new(Some(to.0), to.1)];
    message.subject = Some(subject.to_owned());
    message.flags = [Flag::Seen].into_iter().collect();
    message.rfc_message_id = Some(RfcMessageId::new(format!(
        "<sent.{}@example.test>",
        subject.to_lowercase().replace(' ', ".")
    )));
    let id = MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("a sent message");
    record_correspondents(connection, &message)
        .await
        .expect("its correspondents");
    let thread = ThreadingRepository::new(connection, account.id)
        .thread(&message)
        .await
        .expect("threaded");
    postio_storage::sql::execute(
        connection,
        "INSERT INTO reminders (thread_id, anchor_message_id, set_at, due_at)
         VALUES (?1, ?2, ?3, ?4)",
        postio_storage::sql::bind![
            thread.thread_id.get(),
            id.get(),
            at.timestamp(),
            (at + chrono::Duration::days(7)).timestamp()
        ],
    )
    .await
    .expect("a reminder");
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn the_harbor_reply_lands_as_a_seventh_member_of_the_thread() {
        use postio_storage::sql::scalar;
        let (database, account) = super::seeded(super::Seed::Threads).await;
        let arrival = super::deliver_harbor_reply(&database, account)
            .await
            .expect("the reply is delivered");
        let connection = database.connect().await.expect("a connection");
        let thread: i64 = scalar(
            &connection,
            "SELECT thread_id FROM messages WHERE subject LIKE '%Harbor API draft v3' LIMIT 1",
            (),
        )
        .await
        .expect("harbor");
        assert_eq!(arrival.thread.get(), thread, "filed into Harbor's thread");
        let members: i64 = scalar(
            &connection,
            "SELECT count(*) FROM messages WHERE thread_id = ?1",
            [thread],
        )
        .await
        .expect("a count");
        assert_eq!(members, 7);
        let unread: i64 = scalar(
            &connection,
            "SELECT count(*) FROM messages WHERE thread_id = ?1 AND seen = 0",
            [thread],
        )
        .await
        .expect("a count");
        assert_eq!(unread, 3, "two unread and the reply");
    }

    #[tokio::test]
    async fn the_threads_seed_groups_the_designs_conversations() {
        use postio_storage::sql::scalar;
        let (database, account) = super::seeded(super::Seed::Threads).await;
        let connection = database.connect().await.expect("a connection");
        let count = |subject: &'static str, unread_only: bool| {
            let connection = &connection;
            async move {
                let sql = format!(
                    "SELECT count(*) FROM messages WHERE thread_id IN \
                     (SELECT id FROM threads WHERE subject = lower(?1)){}",
                    if unread_only { " AND seen = 0" } else { "" }
                );
                scalar(connection, &sql, [subject]).await.expect("a count")
            }
        };
        assert_eq!(count("Harbor API draft v3", false).await, 6);
        assert_eq!(count("Harbor API draft v3", true).await, 2);
        assert_eq!(count("Offsite planning", false).await, 23);
        assert_eq!(count("Offsite planning", true).await, 3);
        assert_eq!(count("lost cat", false).await, 41);
        assert_eq!(count("Platform SLOs", false).await, 8);
        assert_eq!(count("Gateway defaults", false).await, 1);
        assert_eq!(
            scalar(
                &connection,
                "SELECT count(DISTINCT thread_id) FROM messages \
                 WHERE subject LIKE '%Harbor API draft v3'",
                (),
            )
            .await
            .expect("a count"),
            1,
            "Harbor is one thread"
        );
        let linked = scalar(
            &connection,
            "SELECT count(*) FROM drafts WHERE in_reply_to_message_id IS NOT NULL AND thread_id = \
             (SELECT id FROM threads WHERE subject = 'harbor api draft v3')",
            (),
        )
        .await
        .expect("a count");
        assert_eq!(linked, 1, "the draft reply is in Harbor's thread");
        let _ = account;
    }

    /// Adding a person someone already owns is refused, so the screenshots panicked.
    #[tokio::test]
    async fn the_compose_people_join_a_demo_that_already_met_them() {
        let (database, account) = super::demo().await;
        super::compose_demo(&database, account).await;
        let connection = database.connect().await.expect("a connection");
        let contacts = postio_storage::repository::ContactRepository::new(&connection);
        for address in ["grace@example.org", "graham@example.net", "ada@example.com"] {
            assert!(
                contacts
                    .by_address(address)
                    .await
                    .expect("a lookup")
                    .is_some(),
                "{address} is nobody"
            );
        }
    }

    #[test]
    fn the_demo_digest_rule_comes_due_on_the_day_the_seed_delivers() {
        let day = super::today()
            .with_timezone(&chrono::Local)
            .format("%A")
            .to_string()
            .to_lowercase();
        let config = super::config();
        assert!(config.contains(&format!("day = \"{day}\"")), "{config}");
    }

    #[tokio::test]
    async fn the_demo_sorts_its_people_and_keeps_the_hostile_senders_out() {
        use postio_storage::sql::scalar;

        let (database, _) = super::demo().await;
        let connection = database.connect().await.expect("a connection");
        let of = |address: &'static str| {
            let connection = &connection;
            async move {
                scalar(
                    connection,
                    "SELECT count(*) FROM contacts c JOIN addresses a ON a.contact_id = c.id
                      WHERE a.address_normalized = ?1 AND c.state = 'live'",
                    postio_storage::sql::bind![address],
                )
                .await
                .expect("a count")
            }
        };
        let automated = scalar(
            &connection,
            "SELECT count(*) FROM contacts c JOIN addresses a ON a.contact_id = c.id
              WHERE a.address_normalized = 'mailer-daemon@mx4.example.net'
                AND c.kind = 'automated'",
            (),
        )
        .await
        .expect("a count");
        assert_eq!(automated, 1, "the bounce sender is automated");
        let companies = scalar(
            &connection,
            "SELECT count(*) FROM contacts WHERE kind = 'company' AND state = 'live'",
            (),
        )
        .await
        .expect("a count");
        assert!(
            companies >= 1,
            "the Companies tab is not empty: {companies}"
        );
        assert_eq!(
            scalar(
                &connection,
                "SELECT count(*) FROM contacts c JOIN addresses a ON a.contact_id = c.id
                  WHERE a.address_normalized = 'office@northfield.example' AND c.kind = 'company'",
                (),
            )
            .await
            .expect("a count"),
            1,
            "the school's office is a company"
        );
        let waiting = ContactRepository::new(&connection)
            .unowned_count()
            .await
            .expect("a count");
        assert_eq!(
            waiting, 0,
            "every address with mail has a person, so the footer never says Building contacts"
        );
        let abuse = scalar(
            &connection,
            "SELECT count(*) FROM contacts c JOIN addresses a ON a.contact_id = c.id
              WHERE a.address_normalized LIKE '%@abuse.example.com' AND c.state = 'live'",
            (),
        )
        .await
        .expect("a count");
        assert_eq!(
            abuse, 0,
            "the corpus's abuse.example.com senders are not demo contacts"
        );
        for hostile in [
            "mallory@sender.invalid",
            "billing@sender.invalid",
            "broken.from@example.com",
        ] {
            assert_eq!(of(hostile).await, 0, "{hostile} is not a demo contact");
        }
    }

    #[tokio::test]
    async fn the_search_seed_is_two_years_of_invented_mail() {
        use postio_storage::sql::{all, scalar};

        assert_eq!(Seed::from_id("search"), Some(Seed::Search));
        assert_eq!(Seed::Search.id(), "search");
        assert!(Seed::ALL.contains(&Seed::Search));

        let (database, account) = seeded(Seed::Search).await;
        let connection = database.connect().await.expect("a connection");
        let account = account.get();

        let messages = scalar(
            &connection,
            "SELECT count(*) FROM messages WHERE account_id = ?1",
            postio_storage::sql::bind![account],
        )
        .await
        .expect("a count");
        assert!(messages >= 400, "only {messages} messages");

        let recent = postio_storage::searches::SearchRepository::new(&connection)
            .recent()
            .await
            .expect("the recent searches");
        let queries: Vec<&str> = recent.iter().map(|each| each.query.as_str()).collect();
        assert_eq!(
            queries,
            [
                "atlas budget",
                "from:ada invoice",
                "has:attachment in:Receipts after:2026-09-01"
            ]
        );

        let oldest = scalar(&connection, "SELECT min(received_at) FROM messages", ())
            .await
            .expect("oldest");
        let newest = scalar(&connection, "SELECT max(received_at) FROM messages", ())
            .await
            .expect("newest");
        let unit = if newest > 100_000_000_000 { 1000 } else { 1 };
        let days = (newest - oldest) / unit / 86_400;
        assert!((700..=740).contains(&days), "spans {days} days");
        let today = today().timestamp();
        let age = (today - newest / unit) / 3_600;
        assert!((0..=24).contains(&age), "newest is {age} hours old");

        let sizes: Vec<i64> = all(
            &connection,
            "SELECT count(*) FROM messages WHERE thread_id IS NOT NULL GROUP BY thread_id",
            (),
            |row| postio_storage::sql::RowExt::int(row, 0),
        )
        .await
        .expect("thread sizes");
        assert!(
            sizes.iter().all(|size| (1..=12).contains(size)),
            "{sizes:?}"
        );
        assert!(sizes.contains(&1) && sizes.contains(&12), "{sizes:?}");

        let outside = scalar(
            &connection,
            "SELECT count(*) FROM addresses WHERE contact_id IS NOT NULL AND address NOT LIKE '%@example.com'",
            (),
        )
        .await
        .expect("contacts");
        assert_eq!(outside, 0, "an address off @example.com");

        for label in ["Atlas", "Harbor", "Receipts"] {
            let tagged = scalar(
                &connection,
                "SELECT count(*) FROM message_labels ml JOIN labels l ON l.id = ml.label_id
                  WHERE l.name = ?1",
                postio_storage::sql::bind![label],
            )
            .await
            .expect("labelled");
            assert!(tagged > 0, "nothing labelled {label}");
        }
        for folder in ["INBOX", "Archive", "Receipts"] {
            let filed = scalar(
                &connection,
                "SELECT count(*) FROM messages m JOIN mailboxes b ON b.id = m.mailbox_id
                  WHERE b.path = ?1",
                postio_storage::sql::bind![folder],
            )
            .await
            .expect("filed");
            assert!(filed > 0, "nothing in {folder}");
        }

        let open = scalar(
            &connection,
            "SELECT count(*) FROM markers WHERE dismissed_at IS NULL",
            (),
        )
        .await
        .expect("markers");
        assert_eq!(open, 2, "two open markers");

        let files = search_files();
        let blobs_dir = tempfile::tempdir().expect("scratch");
        let blobs = postio_storage::BlobStore::open(
            blobs_dir.path().to_path_buf(),
            &postio_storage::test_support::blob_keys(),
        )
        .expect("a blob store");
        let stored = store_search_blobs(&blobs).expect("stored");
        assert_eq!(stored.len(), files.len());
        for extension in ["pdf", "xlsx", "docx", "pptx", "txt"] {
            let file = files
                .iter()
                .find(|file| file.name.ends_with(extension))
                .unwrap_or_else(|| panic!("no .{extension} file"));
            assert!(
                file.name.to_lowercase().contains("atlas"),
                "{} is not about Atlas",
                file.name
            );
            let (_, blob) = stored
                .iter()
                .find(|(name, _)| *name == file.name)
                .expect("its blob");
            let bytes = blobs.get(blob).expect("the bytes");
            assert_eq!(bytes, file.bytes);
            let text = readable_text(extension, &bytes).to_lowercase();
            assert!(text.contains("atlas budget"), "{}: {text}", file.name);
            let rows = scalar(
                &connection,
                "SELECT count(*) FROM attachments WHERE filename = ?1 AND blob_id = ?2",
                postio_storage::sql::bind![file.name, blob.as_str()],
            )
            .await
            .expect("attachment rows");
            assert!(rows > 0, "{} has no stored blob in the store", file.name);
        }
        let undownloaded = scalar(
            &connection,
            "SELECT count(*) FROM attachments WHERE blob_id IS NULL",
            (),
        )
        .await
        .expect("undownloaded");
        assert!(undownloaded > 0);
    }

    fn readable_text(extension: &str, bytes: &[u8]) -> String {
        match extension {
            "xlsx" | "docx" | "pptx" => {
                assert_eq!(&bytes[..4], b"PK\x03\x04");
                String::from_utf8_lossy(bytes).into_owned()
            }
            "pdf" => {
                assert!(bytes.starts_with(b"%PDF-"));
                assert!(bytes.ends_with(b"%%EOF\n"));
                String::from_utf8_lossy(bytes).into_owned()
            }
            _ => String::from_utf8_lossy(bytes).into_owned(),
        }
    }

    use super::*;
    use chrono::{Datelike, TimeZone, Weekday};

    const WEEKDAYS: [(&str, Weekday); 7] = [
        ("Monday", Weekday::Mon),
        ("Tuesday", Weekday::Tue),
        ("Wednesday", Weekday::Wed),
        ("Thursday", Weekday::Thu),
        ("Friday", Weekday::Fri),
        ("Saturday", Weekday::Sat),
        ("Sunday", Weekday::Sun),
    ];

    fn weekday_named(text: &str) -> Option<Weekday> {
        WEEKDAYS
            .iter()
            .filter_map(|(name, day)| text.find(name).map(|at| (at, *day)))
            .min_by_key(|(at, _)| *at)
            .map(|(_, day)| day)
    }

    #[test]
    fn a_marker_date_agrees_with_the_weekday_its_row_names() {
        let today = Utc.with_ymd_and_hms(2026, 6, 2, 16, 9, 0).unwrap();
        let mut checked = 0;
        for row in TODAY {
            let Some(ask) = &row.ask else { continue };
            let (words, date) = match ask {
                Ask::Invite { .. } => {
                    (row.preview, marker(MessageId::new(1), ask, today).starts_at)
                }
                Ask::Todo { sentence, .. } => {
                    (*sentence, marker(MessageId::new(1), ask, today).due_at)
                }
                Ask::Question(_) => continue,
            };
            let Some(named) = weekday_named(words) else {
                continue;
            };
            let date = date.expect("a dated marker").date_naive();
            assert_eq!(
                date.weekday(),
                named,
                "{:?} says {named} but its marker is {date}",
                row.subject
            );
            assert!(
                date > today.date_naive(),
                "{:?} is in the past",
                row.subject
            );
            checked += 1;
        }
        assert!(checked >= 4, "only {checked} rows were checked");
    }
}
