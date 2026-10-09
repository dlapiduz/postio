//! `label:` and `has:action` through the executor (spec 010, US3, S2).
//!
//! `label:` is resolved against `labels.name`, case-insensitively and in
//! every account the scope covers, as `account:` and `in:` are; a name no
//! label carries matches nothing, never everything. `has:action` is an open
//! marker: a `markers` row that has not been dismissed.

use std::collections::BTreeSet;

use chrono::{DateTime, TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::{Account, AccountScope, EmailAddress, Label, Message, MessageId};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::Connection;
use postio_storage::repository::{
    AccountRepository, LabelRepository, MailboxRepository, Marker, MarkerRepository, MarkerSource,
    MessageRepository,
};
use postio_storage::test_support;

fn at(hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 20, hour, 0, 0).unwrap()
}

struct World {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    /// Labelled `Atlas` in the first account.
    atlas_home: MessageId,
    /// Labelled `atlas` (another case) in the second account.
    atlas_work: MessageId,
    /// Labelled `Q3 close` and `Harbor`.
    q3: MessageId,
    /// No label; an open question.
    open: MessageId,
    /// No label; a question the person dismissed.
    dismissed: MessageId,
    /// Labelled `Atlas`, with an open to-do.
    atlas_open: MessageId,
}

impl World {
    fn all(&self) -> BTreeSet<MessageId> {
        [
            self.atlas_home,
            self.atlas_work,
            self.q3,
            self.open,
            self.dismissed,
            self.atlas_open,
        ]
        .into()
    }
}

async fn message(
    connection: &Connection,
    account: &Account,
    subject: &str,
    hour: u32,
) -> MessageId {
    let inbox = match MailboxRepository::new(connection)
        .by_path(account.id, "INBOX")
        .await
        .expect("look up the inbox")
    {
        Some(inbox) => inbox.id,
        None => test_support::mailbox(connection, account, "INBOX").await.id,
    };
    let mut message = Message::new(account.id, inbox, at(hour));
    message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
    message.subject = Some(subject.to_owned());
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create message");
    message.id
}

async fn label(connection: &Connection, account: &Account, name: &str) -> postio_model::LabelId {
    let mut label = Label::new(account.id, name);
    LabelRepository::new(connection)
        .create(&mut label)
        .await
        .expect("create label")
}

async fn mark(connection: &Connection, message: MessageId, dismissed: Option<DateTime<Utc>>) {
    let markers = MarkerRepository::new(connection);
    markers
        .insert(&Marker {
            message,
            kind: postio_model::listing::MarkerKind::Question,
            source: MarkerSource::Detector,
            span: Some((0, 10)),
            excerpt: Some("Can you send the budget?".to_owned()),
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
    if let Some(when) = dismissed {
        markers.dismiss(message, Some(when)).await.expect("dismiss");
    }
}

async fn world() -> World {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");

    let home = test_support::account(&connection).await;
    let mut work = Account::new(
        "Work",
        EmailAddress::new(Some("Test User"), "test@work.example.com"),
    );
    work.incoming.host = "imap.example.com".to_owned();
    work.outgoing.host = "smtp.example.com".to_owned();
    AccountRepository::new(&connection)
        .create(&mut work)
        .await
        .expect("a second account");

    let atlas = label(&connection, &home, "Atlas").await;
    let atlas_lower = label(&connection, &work, "atlas").await;
    let q3_close = label(&connection, &home, "Q3 close").await;
    let harbor = label(&connection, &home, "Harbor").await;
    let labels = LabelRepository::new(&connection);

    let atlas_home = message(&connection, &home, "Atlas kickoff", 1).await;
    labels.attach(atlas_home, atlas).await.expect("attach");
    let atlas_work = message(&connection, &work, "Atlas budget", 2).await;
    labels
        .attach(atlas_work, atlas_lower)
        .await
        .expect("attach");
    let q3 = message(&connection, &home, "Quarter close", 3).await;
    labels.attach(q3, q3_close).await.expect("attach");
    labels.attach(q3, harbor).await.expect("attach");
    let open = message(&connection, &home, "A question", 4).await;
    mark(&connection, open, None).await;
    let dismissed = message(&connection, &home, "An old question", 5).await;
    mark(&connection, dismissed, Some(at(6))).await;
    let atlas_open = message(&connection, &home, "Atlas follow-up", 7).await;
    labels.attach(atlas_open, atlas).await.expect("attach");
    mark(&connection, atlas_open, None).await;

    World {
        _database: database,
        connection,
        atlas_home,
        atlas_work,
        q3,
        open,
        dismissed,
        atlas_open,
    }
}

async fn run(connection: &Connection, query: &str) -> BTreeSet<MessageId> {
    let parsed = parse(query, at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Unified,
        query: &parsed,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Relevance,
    };
    search(connection, &request, at(12))
        .await
        .expect("search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

#[tokio::test]
async fn label_finds_exactly_the_messages_carrying_it_in_any_account() {
    let world = world().await;
    let expected: BTreeSet<_> = [world.atlas_home, world.atlas_work, world.atlas_open].into();
    for query in ["label:atlas", "label:Atlas", "label:ATLAS"] {
        assert_eq!(
            run(&world.connection, query).await,
            expected,
            "{query}: both accounts' Atlas, whatever the case"
        );
    }
    assert_eq!(
        run(&world.connection, r#"label:"Q3 close""#).await,
        [world.q3].into(),
        "a quoted name with a space"
    );
    assert_eq!(
        run(&world.connection, "label:harbor").await,
        [world.q3].into(),
        "a message carrying two labels answers to each"
    );
}

#[tokio::test]
async fn a_label_nobody_has_finds_nothing() {
    let world = world().await;
    assert!(run(&world.connection, "label:nowhere").await.is_empty());
    assert!(
        run(&world.connection, "label:atl").await.is_empty(),
        "a label is named whole, never as a prefix"
    );
}

#[tokio::test]
async fn an_excluded_label_inverts_the_match() {
    let world = world().await;
    let atlas: BTreeSet<_> = [world.atlas_home, world.atlas_work, world.atlas_open].into();
    let expected: BTreeSet<_> = world.all().difference(&atlas).copied().collect();
    assert_eq!(run(&world.connection, "-label:atlas").await, expected);
}

#[tokio::test]
async fn has_action_finds_open_markers_and_not_dismissed_ones() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "has:action").await,
        [world.open, world.atlas_open].into()
    );
    let expected: BTreeSet<_> = world
        .all()
        .into_iter()
        .filter(|id| *id != world.open && *id != world.atlas_open)
        .collect();
    assert!(expected.contains(&world.dismissed));
    assert_eq!(run(&world.connection, "-has:action").await, expected);
}

#[tokio::test]
async fn label_and_has_action_compose() {
    let world = world().await;
    assert_eq!(
        run(&world.connection, "label:atlas has:action").await,
        [world.atlas_open].into()
    );
    assert_eq!(
        run(&world.connection, "label:atlas -has:action budget").await,
        [world.atlas_work].into()
    );
}
