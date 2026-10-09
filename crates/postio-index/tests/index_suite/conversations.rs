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
