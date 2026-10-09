//! `executor::search_conversations` (spec 010, US2): a search answered as
//! conversations, the unit the results view lists.
//!
//! One hit per conversation -- a thread, or a message threading never joined
//! to one -- carrying its best message, how many of its messages matched,
//! and why it ranks where it does (D20). Best match and Newest order the same
//! hits two ways, `offset`/`limit` page them, and `total` counts
//! conversations, not messages.
//!
//! The mail-building helpers here are `pub(crate)` so the facet and budget
//! suites build their corpora the same way.

use chrono::{DateTime, Duration, NaiveDate, TimeZone, Utc};
use postio_index::executor::{ConversationRequest, search_conversations};
use postio_model::{
    Account, AccountScope, Attachment, EmailAddress, Flag, MailboxId, Message, MessageId, Thread,
    ThreadId,
};
use postio_search::parse;
use postio_search::results::{
    ConversationHit, ConversationKey, ConversationOrder, ConversationResults, RankReason, Source,
};
use postio_storage::Connection;
use postio_storage::repository::{MessageRepository, ThreadRepository};
use postio_storage::test_support;

/// The clock every case here runs at.
pub(crate) fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 30, 12, 0, 0).unwrap()
}

pub(crate) fn today() -> NaiveDate {
    now().date_naive()
}

/// One message to file. Everything but the subject has a quiet default.
#[derive(Clone, Default)]
pub(crate) struct Mail {
    pub from: &'static str,
    pub to: &'static [&'static str],
    pub subject: &'static str,
    pub body: &'static str,
    /// How long before [`now`] it arrived.
    pub ago: Duration,
    pub unread: bool,
    pub flagged: bool,
    pub answered: bool,
    pub file: Option<&'static str>,
}

/// Files `mail` in `mailbox`, with its body indexed, and returns it.
pub(crate) async fn file(
    connection: &Connection,
    account: &Account,
    mailbox: MailboxId,
    mail: Mail,
) -> Message {
    let from = if mail.from.is_empty() {
        "ada"
    } else {
        mail.from
    };
    let mut message = Message::new(account.id, mailbox, now() - mail.ago);
    message.from = vec![EmailAddress::new(Some(from), format!("{from}@example.com"))];
    message.to = mail
        .to
        .iter()
        .map(|to| EmailAddress::new(Some(*to), format!("{to}@example.com")))
        .collect();
    message.subject = Some(mail.subject.to_owned());
    if !mail.unread {
        message.flags.insert(Flag::Seen);
    }
    if mail.flagged {
        message.flags.insert(Flag::Flagged);
    }
    if mail.answered {
        message.flags.insert(Flag::Answered);
    }
    if let Some(name) = mail.file {
        let mut attachment = Attachment::new(MessageId::UNASSIGNED, "application/pdf", 2048);
        attachment.filename = Some(name.to_owned());
        message.attachments.push(attachment);
    }
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create a message");
    postio_index::index::index_body(connection, message.id.get(), Some(mail.body))
        .await
        .expect("index its body");
    connection
        .execute(
            "UPDATE messages SET body_state = 'full' WHERE id = ?1",
            [message.id.get()],
        )
        .await
        .expect("mark the body present");
    message
}

/// Puts `messages` in one new thread.
pub(crate) async fn thread(
    connection: &Connection,
    account: &Account,
    messages: &[MessageId],
) -> ThreadId {
    let threads = ThreadRepository::new(connection);
    let mut thread = Thread::new(account.id);
    let id = threads.create(&mut thread).await.expect("a thread");
    for message in messages {
        threads
            .add_message(id, *message)
            .await
            .expect("join the thread");
    }
    id
}

/// An in-memory store with the index installed, one account and its inbox.
pub(crate) async fn store() -> (
    postio_storage::Store,
    postio_storage::Checkout,
    Account,
    MailboxId,
) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    (database, connection, account, inbox)
}

/// Searches `query` as conversations.
pub(crate) async fn conversations(
    connection: &Connection,
    query: &str,
    order: ConversationOrder,
    offset: u32,
    limit: u32,
) -> ConversationResults {
    let parsed = parse(query, today());
    let request = ConversationRequest {
        account: AccountScope::Unified,
        query: &parsed,
        order,
        offset,
        limit,
        today: today(),
    };
    search_conversations(connection, &request, now())
        .await
        .expect("a conversation search")
}

fn hit(results: &ConversationResults, key: ConversationKey) -> &ConversationHit {
    results
        .hits
        .iter()
        .find(|hit| hit.key == key)
        .unwrap_or_else(|| panic!("no hit for {key:?} in {:#?}", results.hits))
}

fn keys(results: &ConversationResults) -> Vec<ConversationKey> {
    results.hits.iter().map(|hit| hit.key).collect()
}

#[tokio::test]
async fn a_thread_whose_messages_match_is_one_hit() {
    let (_database, connection, account, inbox) = store().await;
    let at = |hours: i64| Duration::hours(hours);
    let kickoff = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Atlas kickoff",
            body: "The atlas plan for the quarter, atlas first.",
            ago: at(50),
            ..Mail::default()
        },
    )
    .await;
    let reply = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Re: kickoff",
            body: "Agreed on atlas.",
            ago: at(40),
            ..Mail::default()
        },
    )
    .await;
    let later = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Re: kickoff",
            body: "One more note on atlas.",
            ago: at(30),
            ..Mail::default()
        },
    )
    .await;
    // The thread's newest message says nothing about it.
    let unrelated = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Re: kickoff",
            body: "Lunch on Friday?",
            ago: at(20),
            ..Mail::default()
        },
    )
    .await;
    let conversation = thread(
        &connection,
        &account,
        &[kickoff.id, reply.id, later.id, unrelated.id],
    )
    .await;
    let lone = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Notes",
            body: "atlas, briefly",
            ago: at(60),
            ..Mail::default()
        },
    )
    .await;

    let results = conversations(&connection, "atlas", ConversationOrder::BestMatch, 0, 10).await;

    assert_eq!(results.total, 2, "two conversations, whatever their sizes");
    assert!(!results.capped);
    assert_eq!(results.hits.len(), 2);
    let threaded = hit(&results, ConversationKey::Thread(conversation));
    assert_eq!(threaded.messages, 4, "the badge counts the whole thread");
    assert_eq!(
        threaded.best, kickoff.id,
        "the best match is the message saying it in its subject too"
    );
    assert_eq!(threaded.subject.as_deref(), Some("Atlas kickoff"));
    assert_eq!(threaded.mailbox_id, inbox);
    assert_eq!(
        threaded.newest_match, later.received_at,
        "the newest *matching* message, not the thread's newest"
    );
    assert_eq!(threaded.reasons.last(), Some(&RankReason::Matches(3)));

    let alone = hit(&results, ConversationKey::Lone(lone.id));
    assert_eq!(alone.best, lone.id);
    assert_eq!(
        alone.messages, 1,
        "a message on its own is a conversation of one"
    );
    assert_eq!(alone.reasons, [RankReason::Matches(1)]);
    assert_eq!(
        alone.from.as_ref().map(|from| from.address.as_str()),
        Some("ada@example.com")
    );
}

#[tokio::test]
async fn the_reasons_say_why_a_conversation_ranks_where_it_does() {
    let (_database, connection, account, inbox) = store().await;
    let day = |days: i64| Duration::days(days);
    let replied = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Dock schedule",
            body: "The harbor opens at nine.",
            answered: true,
            ago: day(1),
            ..Mail::default()
        },
    )
    .await;
    let flagged = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Tides",
            body: "High tide in the harbor at noon.",
            flagged: true,
            ago: day(2),
            ..Mail::default()
        },
    )
    .await;
    let in_subject = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Harbor permits",
            body: "Forms attached later.",
            ago: day(3),
            ..Mail::default()
        },
    )
    .await;
    let in_file = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Plans",
            body: "See the file.",
            file: Some("harbor-plan.pdf"),
            ago: day(4),
            ..Mail::default()
        },
    )
    .await;
    let frequent = file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "grace",
            subject: "Weekend",
            body: "Sailing out of the harbor.",
            ago: day(5),
            ..Mail::default()
        },
    )
    .await;
    // The person has written to Grace a dozen times: two-way frequency
    // (D21) is what makes a sender frequent.
    connection
        .execute(
            "INSERT INTO correspondents (address_id, sent_count, last_sent_at)
             SELECT id, 12, 0 FROM addresses WHERE address_normalized = 'grace@example.com'",
            (),
        )
        .await
        .expect("a correspondent");

    let results = conversations(&connection, "harbor", ConversationOrder::Newest, 0, 10).await;
    let reasons = |id: MessageId| hit(&results, ConversationKey::Lone(id)).reasons.clone();
    let sources = |id: MessageId| -> Vec<Source> {
        hit(&results, ConversationKey::Lone(id))
            .matches
            .iter()
            .map(|found| found.source.clone())
            .collect()
    };

    assert_eq!(
        reasons(replied.id),
        [RankReason::Replied, RankReason::Matches(1)]
    );
    assert_eq!(
        reasons(flagged.id),
        [RankReason::Flagged, RankReason::Matches(1)]
    );
    assert_eq!(
        reasons(in_subject.id),
        [RankReason::InSubject, RankReason::Matches(1)]
    );
    assert_eq!(
        reasons(in_file.id),
        [RankReason::InFileName, RankReason::Matches(1)]
    );
    assert_eq!(
        reasons(frequent.id),
        [RankReason::FrequentSender, RankReason::Matches(1)]
    );

    assert_eq!(sources(replied.id), [Source::Body]);
    assert_eq!(sources(in_subject.id), [Source::Subject]);
    let attachment = in_file.attachments[0].id;
    assert_eq!(
        sources(in_file.id),
        [Source::FileName {
            attachment,
            name: "harbor-plan.pdf".to_owned()
        }]
    );
    let found = &hit(&results, ConversationKey::Lone(replied.id)).matches[0];
    assert_eq!(found.when, Some(replied.received_at));
    assert_eq!(found.passage, None, "passages are a read of their own");
}

#[tokio::test]
async fn best_match_and_newest_order_the_same_hits_two_ways() {
    let (_database, connection, account, inbox) = store().await;
    // About it, and an hour older.
    let about = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Budget review",
            body: "The budget, line by line: the budget for travel, the budget for tools.",
            ago: Duration::hours(3),
            ..Mail::default()
        },
    )
    .await;
    // Mentions it in passing, and newer.
    let passing = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Weekly notes",
            body: "Several things happened this week, among them a remark about the \
                   budget, a long discussion of the offsite, the menu for Friday, the \
                   parking situation, and who is bringing the projector next time.",
            ago: Duration::hours(2),
            ..Mail::default()
        },
    )
    .await;

    let best = conversations(&connection, "budget", ConversationOrder::BestMatch, 0, 10).await;
    let newest = conversations(&connection, "budget", ConversationOrder::Newest, 0, 10).await;

    assert_eq!(
        keys(&best),
        [
            ConversationKey::Lone(about.id),
            ConversationKey::Lone(passing.id)
        ],
        "best match leads with the message about it"
    );
    assert_eq!(
        keys(&newest),
        [
            ConversationKey::Lone(passing.id),
            ConversationKey::Lone(about.id)
        ],
        "newest is date order, whatever the match"
    );
    assert_eq!(best.total, newest.total);
}

#[tokio::test]
async fn offset_and_limit_page_through_conversations() {
    let (_database, connection, account, inbox) = store().await;
    let mut lone = Vec::new();
    for nth in 0..5 {
        lone.push(
            file(
                &connection,
                &account,
                inbox,
                Mail {
                    subject: "Invoice",
                    body: "invoice attached",
                    ago: Duration::days(nth),
                    ..Mail::default()
                },
            )
            .await
            .id,
        );
    }
    // Two more matching messages in one thread: one conversation, not two.
    let first = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Invoice question",
            body: "about the invoice",
            ago: Duration::days(10),
            ..Mail::default()
        },
    )
    .await;
    let second = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Re: Invoice question",
            body: "the invoice is paid",
            ago: Duration::days(9),
            ..Mail::default()
        },
    )
    .await;
    let threaded = thread(&connection, &account, &[first.id, second.id]).await;

    let page = |offset, limit| {
        conversations(
            &connection,
            "invoice",
            ConversationOrder::Newest,
            offset,
            limit,
        )
    };
    let all = page(0, 10).await;
    assert_eq!(all.total, 6, "six conversations from seven messages");
    let mut expected: Vec<ConversationKey> =
        lone.iter().map(|id| ConversationKey::Lone(*id)).collect();
    expected.push(ConversationKey::Thread(threaded));
    assert_eq!(keys(&all), expected);

    let second_page = page(2, 2).await;
    assert_eq!(keys(&second_page), expected[2..4]);
    assert_eq!(second_page.total, 6, "the total is the whole match's");
    let last = page(4, 10).await;
    assert_eq!(keys(&last), expected[4..]);
    assert!(page(6, 10).await.hits.is_empty(), "past the end, nothing");
}

#[tokio::test]
async fn it_says_how_many_messages_it_looked_through() {
    let (_database, connection, account, inbox) = store().await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    for (nth, folder) in [inbox, inbox, archive, archive, archive]
        .into_iter()
        .enumerate()
    {
        file(
            &connection,
            &account,
            folder,
            Mail {
                subject: if nth == 0 { "Ferry times" } else { "Other" },
                body: "nothing much",
                ago: Duration::days(nth as i64),
                ..Mail::default()
            },
        )
        .await;
    }

    let results = conversations(&connection, "ferry", ConversationOrder::BestMatch, 0, 10).await;
    assert_eq!(results.total, 1);
    assert_eq!(
        results.messages_searched, 5,
        "every message the search could have found, matched or not"
    );
    assert!(results.corpus_complete);
    // No attachment is on this machine, so none is left unread (step 9;
    // `contents_are_incomplete_while_a_downloaded_attachment_is_unread`).
    assert!(results.contents_complete);
}

#[tokio::test]
async fn a_filter_narrows_the_hits_without_losing_their_order() {
    let (_database, connection, account, inbox) = store().await;
    let about = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Budget review",
            body: "The budget, line by line: the budget for travel, the budget for tools.",
            ago: Duration::hours(3),
            ..Mail::default()
        },
    )
    .await;
    let passing = file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Weekly notes",
            body: "Several things happened this week, among them a remark about the \
                   budget, a long discussion of the offsite, the menu for Friday, the \
                   parking situation, and who is bringing the projector next time.",
            ago: Duration::hours(2),
            ..Mail::default()
        },
    )
    .await;
    let elsewhere = file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "grace",
            subject: "Budget",
            body: "Grace's budget",
            ago: Duration::hours(1),
            ..Mail::default()
        },
    )
    .await;
    let ada = [
        ConversationKey::Lone(about.id),
        ConversationKey::Lone(passing.id),
    ];

    for query in [
        "budget from:ada",
        "budget -from:grace",
        "from:ada budget",
        "budget -grace",
    ] {
        let best = conversations(&connection, query, ConversationOrder::BestMatch, 0, 10).await;
        assert_eq!(keys(&best), ada, "{query:?}, best match first");
        assert_eq!(best.total, 2, "{query:?}");
        let newest = conversations(&connection, query, ConversationOrder::Newest, 0, 10).await;
        assert_eq!(keys(&newest), [ada[1], ada[0]], "{query:?}, newest first");
    }
    let only = conversations(
        &connection,
        "from:grace",
        ConversationOrder::BestMatch,
        0,
        10,
    )
    .await;
    assert_eq!(keys(&only), [ConversationKey::Lone(elsewhere.id)]);
    let not = conversations(&connection, "-from:ada", ConversationOrder::Newest, 0, 10).await;
    assert_eq!(keys(&not), [ConversationKey::Lone(elsewhere.id)]);
}

/// A message filed in two folders is one content (#1780): the index holds it
/// once, and a conversation search counts it once, as `search` does, on
/// every walk -- the free text, a set, a refusal, the folder alone -- and
/// so does the count a way out of no results offers.
#[tokio::test]
async fn a_message_filed_twice_is_one_match() {
    let (_database, connection, account, inbox) = store().await;
    let mut archive = postio_model::Mailbox::new(account.id, "Archive", Some('/'));
    archive.role = postio_model::MailboxRole::Archive;
    postio_storage::repository::MailboxRepository::new(&connection)
        .create(&mut archive)
        .await
        .expect("an archive");
    let messages = MessageRepository::new(&connection);
    let mut first = Message::new(account.id, inbox, now() - Duration::hours(2));
    first.from = vec![EmailAddress::new(Some("ada"), "ada@example.com".to_owned())];
    first.subject = Some("Nebula survey".to_owned());
    first.server.content_identity = Some(postio_model::ContentIdentity::new("jmap-email", "e-1"));
    messages.create(&mut first).await.expect("the inbox copy");
    let mut second = first.clone();
    second.mailbox_id = archive.id;
    messages
        .create(&mut second)
        .await
        .expect("the archive copy");
    postio_index::index::index_body(&connection, first.id.get(), Some("the nebula, mapped"))
        .await
        .expect("its body");

    for query in [
        "nebula",
        "nebula from:ada",
        "from:ada",
        "nebula -from:grace",
        "-from:grace",
        "is:unread",
    ] {
        let results = conversations(&connection, query, ConversationOrder::BestMatch, 0, 10).await;
        assert_eq!(results.total, 1, "{query:?}: {:#?}", results.hits);
        assert_eq!(
            keys(&results),
            [ConversationKey::Lone(first.id)],
            "{query:?}"
        );
        assert_eq!(results.hits[0].messages, 1, "{query:?}");
        let counts = postio_index::executor::relaxation_counts(
            &connection,
            AccountScope::Unified,
            &[postio_search::relax::Relaxation {
                loosen: postio_search::relax::Loosen::Drop { token: 0 },
                query: query.to_owned(),
            }],
            today(),
        )
        .await
        .expect("its count");
        assert_eq!(counts, [1], "{query:?}, counted");
    }
}

// ---------------------------------------------------------------------------
// Attachment contents (spec 010 step 9, US8, T126)
// ---------------------------------------------------------------------------

/// Gives `message`'s attachment named `name` its bytes on this machine, as
/// a download would: the indexer's queue now holds it.
pub(crate) async fn downloaded(
    connection: &Connection,
    message: MessageId,
    name: &str,
    mime: &str,
) {
    connection
        .execute(
            "UPDATE attachments SET blob_id = ?1, mime_type = ?2
              WHERE message_id = ?3 AND filename = ?4",
            (format!("blob-{name}"), mime, message.get(), name),
        )
        .await
        .expect("the blob is stored");
}

/// The attachment of `message` named `name`.
pub(crate) async fn attachment_named(
    connection: &Connection,
    message: MessageId,
    name: &str,
) -> postio_model::AttachmentId {
    postio_storage::sql::one(
        connection,
        "SELECT id FROM attachments WHERE message_id = ?1 AND filename = ?2",
        (message.get(), name),
        |row| {
            use postio_storage::sql::RowExt as _;
            Ok(postio_model::AttachmentId::new(row.col(0)?))
        },
    )
    .await
    .expect("the attachment")
}

/// What the extractor read from a budget sheet: "kestrel" only on row 14 of
/// "Summary", nowhere in the mail itself.
pub(crate) fn budget_sheet() -> postio_extract::Extracted {
    let row = |row: u32, text: &str| postio_extract::Unit {
        location: postio_extract::Location::Sheet {
            name: "Summary".to_owned(),
            row,
        },
        text: text.to_owned(),
    };
    postio_extract::Extracted {
        units: vec![
            row(1, "Line item | Q3 | Q4"),
            row(3, "Travel | 1,200 | 900"),
            row(14, "Kestrel survey | 4,500 | 4,800"),
        ],
        outcome: postio_extract::Outcome::Complete,
    }
}

/// A mail whose only mention of "kestrel" is in its spreadsheet, read.
pub(crate) async fn mail_with_budget_sheet(
    connection: &Connection,
    account: &Account,
    inbox: MailboxId,
) -> (Message, postio_model::AttachmentId) {
    let message = file(
        connection,
        account,
        inbox,
        Mail {
            subject: "Q3 numbers",
            body: "The sheet is attached, as promised.",
            file: Some("Atlas-Q3-budget.xlsx"),
            ago: Duration::days(2),
            ..Mail::default()
        },
    )
    .await;
    downloaded(
        connection,
        message.id,
        "Atlas-Q3-budget.xlsx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
    )
    .await;
    let attachment = attachment_named(connection, message.id, "Atlas-Q3-budget.xlsx").await;
    (message, attachment)
}

#[tokio::test]
async fn a_word_only_inside_an_attachment_finds_its_conversation_where_it_is() {
    let (_database, connection, account, inbox) = store().await;
    let (message, attachment) = mail_with_budget_sheet(&connection, &account, inbox).await;
    assert!(
        postio_index::index::index_attachment_text(&connection, attachment, &budget_sheet())
            .await
            .expect("index its text"),
        "the attachment is there to index"
    );

    let results = conversations(&connection, "kestrel", ConversationOrder::BestMatch, 0, 10).await;
    assert_eq!(
        keys(&results),
        [ConversationKey::Lone(message.id)],
        "the word is only in the sheet, and the sheet finds its mail"
    );
    let hit = &results.hits[0];
    assert_eq!(
        hit.matches
            .iter()
            .map(|found| found.source.clone())
            .collect::<Vec<_>>(),
        [Source::FileContent {
            attachment,
            name: "Atlas-Q3-budget.xlsx".to_owned(),
            location: postio_search::results::Location::Sheet {
                name: "Summary".to_owned(),
                row: 14,
            },
        }],
        "where in the file: the sheet and the row"
    );
    assert!(
        hit.reasons.contains(&RankReason::InFileName),
        "a file found it: {:?}",
        hit.reasons
    );
    assert!(
        results.contents_complete,
        "every downloaded attachment has been read"
    );

    // Newest finds it too: the arm is the match's, not the ranking's.
    let newest = conversations(&connection, "kestrel", ConversationOrder::Newest, 0, 10).await;
    assert_eq!(keys(&newest), [ConversationKey::Lone(message.id)]);
    // And beside a set, which walks the match apart from `messages`.
    let narrowed = conversations(
        &connection,
        "kestrel from:ada",
        ConversationOrder::BestMatch,
        0,
        10,
    )
    .await;
    assert_eq!(keys(&narrowed), [ConversationKey::Lone(message.id)]);
}

#[tokio::test]
async fn contents_are_incomplete_while_a_downloaded_attachment_is_unread() {
    let (_database, connection, account, inbox) = store().await;
    let (_, attachment) = mail_with_budget_sheet(&connection, &account, inbox).await;

    let before = conversations(&connection, "numbers", ConversationOrder::BestMatch, 0, 10).await;
    assert!(
        !before.contents_complete,
        "a downloaded sheet nobody has read yet: the search cannot say it looked inside"
    );

    postio_index::index::index_attachment_text(&connection, attachment, &budget_sheet())
        .await
        .expect("index its text");
    let after = conversations(&connection, "numbers", ConversationOrder::BestMatch, 0, 10).await;
    assert!(after.contents_complete, "read now");
}

#[tokio::test]
async fn an_attachment_never_downloaded_leaves_the_contents_complete() {
    let (_database, connection, account, inbox) = store().await;
    // The file is named, its bytes are on the server: nothing here can read
    // it, and nothing asks for it (FR-050).
    file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Q3 numbers",
            file: Some("Atlas-Q3-budget.xlsx"),
            ..Mail::default()
        },
    )
    .await;
    let results = conversations(&connection, "numbers", ConversationOrder::BestMatch, 0, 10).await;
    assert!(results.contents_complete);
}

#[tokio::test]
async fn gtk_search_does_not_reach_attachment_contents() {
    // D11: the old path keeps its corpus until Linux adopts this search.
    let (_database, connection, account, inbox) = store().await;
    let (_, attachment) = mail_with_budget_sheet(&connection, &account, inbox).await;
    postio_index::index::index_attachment_text(&connection, attachment, &budget_sheet())
        .await
        .expect("index its text");
    let parsed = parse("kestrel", today());
    let found = postio_index::executor::search(
        &connection,
        &postio_index::SearchRequest {
            account: AccountScope::Unified,
            query: &parsed,
            scope: postio_search::facets::Scope::AllMail,
            limit: 50,
            order: postio_search::ResultOrder::Relevance,
        },
        now(),
    )
    .await
    .expect("GTK's search");
    assert!(found.hits.is_empty(), "{:#?}", found.hits);
}

// ---------------------------------------------------------------------------
// The Files tab (spec 010 step 9, US8, T128)
// ---------------------------------------------------------------------------

async fn files(connection: &Connection, query: &str) -> Vec<postio_search::results::FileHit> {
    let parsed = parse(query, today());
    postio_index::executor::files(
        connection,
        &ConversationRequest {
            account: AccountScope::Unified,
            query: &parsed,
            order: ConversationOrder::Newest,
            offset: 0,
            limit: 50,
            today: today(),
        },
    )
    .await
    .expect("the files")
}

#[tokio::test]
async fn the_files_tab_has_one_card_per_matching_attachment_with_its_match() {
    let (_database, connection, account, inbox) = store().await;
    // The sheet says "kestrel" on row 14 of "Summary".
    let (sheet_mail, sheet) = mail_with_budget_sheet(&connection, &account, inbox).await;
    postio_index::index::index_attachment_text(&connection, sheet, &budget_sheet())
        .await
        .expect("index its text");
    // A file whose name says it, never downloaded: a card by its name.
    let plan_mail = file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "grace",
            subject: "The plan",
            body: "Attached.",
            file: Some("kestrel-plan.pdf"),
            ago: Duration::days(1),
            ..Mail::default()
        },
    )
    .await;
    let plan = attachment_named(&connection, plan_mail.id, "kestrel-plan.pdf").await;
    // The body says it, the file does not: the mail matches, the file is
    // no card.
    file(
        &connection,
        &account,
        inbox,
        Mail {
            subject: "Field notes",
            body: "A kestrel over the car park again.",
            file: Some("notes.pdf"),
            ago: Duration::days(3),
            ..Mail::default()
        },
    )
    .await;

    let found = files(&connection, "kestrel").await;
    assert_eq!(
        found
            .iter()
            .map(|hit| (hit.attachment, hit.message, hit.name.as_str()))
            .collect::<Vec<_>>(),
        [
            (plan, plan_mail.id, "kestrel-plan.pdf"),
            (sheet, sheet_mail.id, "Atlas-Q3-budget.xlsx"),
        ],
        "newest first; the file whose mail only says it in the body is not a card"
    );

    let by_name = &found[0];
    assert_eq!(by_name.subject.as_deref(), Some("The plan"));
    assert_eq!(
        by_name.from.as_ref().map(|from| from.address.as_str()),
        Some("grace@example.com")
    );
    assert_eq!(by_name.size, 2048);
    let matched = by_name.matched.as_ref().expect("its name matched");
    assert_eq!(
        matched.source,
        Source::FileName {
            attachment: plan,
            name: "kestrel-plan.pdf".to_owned()
        }
    );

    let by_contents = &found[1];
    assert!(by_contents.mime_type.contains("spreadsheetml"));
    let matched = by_contents.matched.as_ref().expect("its contents matched");
    assert_eq!(
        matched.source,
        Source::FileContent {
            attachment: sheet,
            name: "Atlas-Q3-budget.xlsx".to_owned(),
            location: postio_search::results::Location::Sheet {
                name: "Summary".to_owned(),
                row: 14,
            },
        }
    );
    let passage = matched.passage.as_ref().expect("the matching line");
    assert!(passage.text.starts_with("Kestrel survey"), "{passage:?}");
    assert_eq!(
        passage
            .ranges
            .iter()
            .map(|range| passage.text[range.clone()].to_lowercase())
            .collect::<Vec<_>>(),
        ["kestrel"]
    );
}

#[tokio::test]
async fn with_no_words_every_attachment_of_the_matched_mail_is_a_card() {
    let (_database, connection, account, inbox) = store().await;
    let (_, sheet) = mail_with_budget_sheet(&connection, &account, inbox).await;
    file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "grace",
            subject: "Not hers",
            file: Some("other.pdf"),
            ..Mail::default()
        },
    )
    .await;
    let found = files(&connection, "from:ada").await;
    assert_eq!(
        found.iter().map(|hit| hit.attachment).collect::<Vec<_>>(),
        [sheet]
    );
    assert_eq!(found[0].matched, None, "no word to have matched");
}
