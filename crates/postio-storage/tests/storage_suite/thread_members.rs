//! Reading a thread's members: each message's own line, who it answers, and
//! the unsent reply (spec 013 R6). One statement over `idx_messages_thread`
//! and one over `idx_drafts_thread`, however long the thread.

use chrono::{DateTime, TimeZone, Utc};
use postio_model::listing::MarkerKind;
use postio_model::{
    AccountId, Draft, EmailAddress, Line, MailboxId, Message, MessageBody, MessageId, RfcMessageId,
    Thread, ThreadId,
};
use postio_storage::Connection;
use postio_storage::repository::{
    DraftRepository, Marker, MarkerRepository, MarkerSource, MessageRepository, ThreadRepository,
};
use postio_storage::test_support::{self, counting::counted_async};

fn at(minutes: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 21, 9, 0, 0).unwrap() + chrono::TimeDelta::minutes(minutes)
}

struct Seed<'a> {
    connection: &'a Connection,
    account: AccountId,
    inbox: MailboxId,
    thread: ThreadId,
}

impl Seed<'_> {
    /// A member received at `minutes`, with Message-ID `<{id}@example.com>`
    /// and, when given, an `In-Reply-To`.
    async fn member(
        &self,
        id: &str,
        replying_to: Option<&str>,
        sender: &str,
        minutes: i64,
    ) -> MessageId {
        let mut message = Message::new(self.account, self.inbox, at(minutes));
        message.rfc_message_id = Some(RfcMessageId::new(format!("<{id}@example.com>")));
        message.in_reply_to =
            replying_to.map(|id| RfcMessageId::new(format!("<{id}@example.com>")));
        message.from = vec![EmailAddress::new(
            Some(sender),
            format!("{sender}@example.com"),
        )];
        message.preview = Some(format!("Preview of {id}"));
        let id = MessageRepository::new(self.connection)
            .create(&mut message)
            .await
            .expect("a message");
        ThreadRepository::new(self.connection)
            .add_message(self.thread, id)
            .await
            .expect("added to the thread");
        id
    }
}

async fn seeded(connection: &Connection) -> Seed<'_> {
    let (account, inbox) = test_support::account_with_inbox(connection).await;
    let mut thread = Thread::new(account.id);
    thread.subject = Some("Harbor API draft".to_owned());
    ThreadRepository::new(connection)
        .create(&mut thread)
        .await
        .expect("a thread");
    Seed {
        connection,
        account: account.id,
        inbox,
        thread: thread.id,
    }
}

#[tokio::test]
async fn the_members_come_in_date_order_with_the_line_each_has_to_show() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let seed = seeded(&connection).await;
    // Created newest first, so only the sort can put them right.
    let third = seed.member("c", Some("b"), "ada", 30).await;
    let first = seed.member("a", None, "ben", 10).await;
    let second = seed.member("b", Some("a"), "test", 20).await;
    let messages = MessageRepository::new(&connection);
    messages
        .set_own_line(first, "Here is the draft.")
        .await
        .expect("a line");
    messages.set_own_line(second, "").await.expect("all quote");

    let members = ThreadRepository::new(&connection)
        .members(seed.thread)
        .await
        .expect("members");

    let ids: Vec<_> = members.iter().map(|member| member.id).collect();
    assert_eq!(ids, [first, second, third]);
    assert_eq!(members[0].line, Line::Own("Here is the draft.".to_owned()));
    assert_eq!(members[1].line, Line::Quoted);
    assert_eq!(
        members[2].line,
        Line::Preview("Preview of c".to_owned()),
        "no body yet: the preview"
    );
    assert_eq!(members[0].at, at(10));
    assert_eq!(members[0].from.as_ref().unwrap().address, "ben@example.com");
    assert!(!members[0].own);
    assert!(members[1].own, "sent from the account's address");
}

#[tokio::test]
async fn a_preview_loses_the_attribution_it_runs_into() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let seed = seeded(&connection).await;
    let id = seed.member("a", None, "ben", 10).await;
    sqlx_preview(
        &connection,
        id,
        "Sounds good. On Mon, 21 Sep 2026, Ana wrote: > The draft",
    )
    .await;

    let members = ThreadRepository::new(&connection)
        .members(seed.thread)
        .await
        .expect("members");

    assert_eq!(members[0].line, Line::Preview("Sounds good.".to_owned()));
}

async fn sqlx_preview(connection: &Connection, id: MessageId, preview: &str) {
    postio_storage::sql::execute(
        connection,
        "UPDATE messages SET preview = ?2 WHERE id = ?1",
        postio_storage::sql::bind![id.get(), preview],
    )
    .await
    .expect("a preview");
}

#[tokio::test]
async fn answers_is_set_only_for_a_parent_that_is_not_the_previous_member() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let seed = seeded(&connection).await;
    seed.member("a", None, "ana", 10).await;
    seed.member("b", Some("a"), "ben", 20).await;
    seed.member("c", Some("b"), "ada", 30).await;
    // Names the first message, case-insensitively: not the one before it.
    let mut fourth = Message::new(seed.account, seed.inbox, at(40));
    fourth.rfc_message_id = Some(RfcMessageId::new("<d@example.com>"));
    fourth.in_reply_to = Some(RfcMessageId::new("<A@Example.com>"));
    let fourth = MessageRepository::new(&connection)
        .create(&mut fourth)
        .await
        .expect("a message");
    ThreadRepository::new(&connection)
        .add_message(seed.thread, fourth)
        .await
        .expect("added");
    // Names a message that is not here.
    seed.member("e", Some("elsewhere"), "ana", 50).await;

    let members = ThreadRepository::new(&connection)
        .members(seed.thread)
        .await
        .expect("members");

    let answers: Vec<_> = members.iter().map(|member| member.answers).collect();
    assert_eq!(answers, [None, None, None, Some(0), None]);
}

#[tokio::test]
async fn an_open_marker_shows_and_a_dismissed_one_does_not() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let seed = seeded(&connection).await;
    let asks = seed.member("a", None, "ana", 10).await;
    let done = seed.member("b", Some("a"), "ben", 20).await;
    let markers = MarkerRepository::new(&connection);
    for message in [asks, done] {
        markers
            .insert(&Marker {
                message,
                kind: MarkerKind::Question,
                source: MarkerSource::Detector,
                span: Some((0, 5)),
                excerpt: Some("Can you?".to_owned()),
                starts_at: None,
                ends_at: None,
                due_at: None,
                invite: None,
                invite_state: None,
                answer: None,
                dismissed_at: None,
            })
            .await
            .expect("a marker");
    }
    markers
        .dismiss(done, Some(at(25)))
        .await
        .expect("dismissed");

    let members = ThreadRepository::new(&connection)
        .members(seed.thread)
        .await
        .expect("members");

    assert_eq!(
        members
            .iter()
            .map(|member| member.marker)
            .collect::<Vec<_>>(),
        [true, false]
    );
}

#[tokio::test]
async fn the_unsent_reply_is_found_by_the_thread_it_is_in() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let seed = seeded(&connection).await;
    seed.member("a", None, "ana", 10).await;
    let threads = ThreadRepository::new(&connection);
    assert_eq!(threads.draft(seed.thread).await.expect("a read"), None);

    let mut draft = Draft::new(seed.account);
    draft.thread_id = Some(seed.thread);
    draft.body = MessageBody {
        text: Some("Thanks Ana,\n\nI will look at it tomorrow.".to_owned()),
        html: None,
    };
    draft.created_at = at(60);
    draft.updated_at = at(61);
    // Another thread's draft is not this one's.
    let mut elsewhere = Draft::new(seed.account);
    elsewhere.updated_at = at(90);
    let drafts = DraftRepository::new(&connection);
    let id = drafts.save(&mut draft).await.expect("a draft");
    drafts.save(&mut elsewhere).await.expect("a draft");

    let found = threads
        .draft(seed.thread)
        .await
        .expect("a read")
        .expect("the draft");

    assert_eq!(found.id, id);
    assert_eq!(found.first_words, "Thanks Ana, I will look at it tomorrow.");
    assert_eq!(found.updated_at, at(61));
}

/// One thread of `count` messages, each answering the one before it, and a
/// draft: what reading it costs.
async fn cost_of_reading(count: usize) -> (postio_storage::test_support::counting::Counts, usize) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    let seed = seeded(&connection).await;
    for n in 0..count {
        let id = format!("m{n}");
        // Every third message names the first: some `answers` to resolve.
        let parent = match n {
            0 => None,
            n if n % 3 == 0 => Some("m0".to_owned()),
            n => Some(format!("m{}", n - 1)),
        };
        let message = seed.member(&id, parent.as_deref(), "ana", n as i64).await;
        MessageRepository::new(&connection)
            .set_own_line(message, "A line")
            .await
            .expect("a line");
    }
    let mut draft = Draft::new(seed.account);
    draft.thread_id = Some(seed.thread);
    DraftRepository::new(&connection)
        .save(&mut draft)
        .await
        .expect("a draft");

    let mut read = 0;
    let counts = counted_async(|| async {
        let threads = ThreadRepository::new(&connection);
        read = threads.members(seed.thread).await.expect("members").len();
        threads.draft(seed.thread).await.expect("draft");
    })
    .await;
    (counts, read)
}

#[tokio::test]
async fn reading_a_thread_costs_the_same_statements_at_two_members_and_five_hundred() {
    let (small, small_members) = cost_of_reading(2).await;
    let (large, large_members) = cost_of_reading(500).await;

    assert_eq!((small_members, large_members), (2, 500));
    assert_eq!(
        small.statements, large.statements,
        "statements: {small:?} against {large:?}"
    );
    // A row per member and one for the draft, and nothing read twice. No
    // counter sees trigger firings, so a trigger on `messages` reads cannot
    // be asserted here; none fires on a SELECT.
    assert_eq!(small.rows, small_members + 1, "{small:?}");
    assert_eq!(large.rows, large_members + 1, "{large:?}");
    // `compiles` is not compared: the statement cache outlives a store, so
    // whichever size runs second compiles nothing.
}
