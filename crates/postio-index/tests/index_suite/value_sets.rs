//! Either of several values, `from:{ada tomas}`, through every way the
//! store answers a query (spec 010, D26, FR-006).
//!
//! A set holds when any of its values does, and negated when none does.
//! GTK's `executor::search` answers it as a condition (`filter_condition`),
//! the conversation search as one set of message ids (`Plan::build_sets`),
//! and a way out of no results counts it with the same sets; all three must
//! give the answer each value's own clause gives, ORed.

use std::collections::BTreeSet;

use chrono::Duration;
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, Label, MailboxRole, MessageId};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_search::relax::{Loosen, Relaxation};
use postio_search::results::{ConversationKey, ConversationOrder};
use postio_storage::Connection;
use postio_storage::sql::RowExt as _;
use postio_storage::repository::{LabelRepository, MailboxRepository};

use crate::conversations::{Mail, conversations, file, now, store, today};

struct World {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    /// From Ada, in the inbox, labelled Atlas.
    ada: MessageId,
    /// From Tomás, archived, labelled Harbor.
    tomas: MessageId,
    /// From Bo, in the inbox, no label.
    bo: MessageId,
    /// From Ada, in the trash: out of All mail unless a folder is named.
    binned: MessageId,
}

async fn world() -> World {
    let (database, connection, account, inbox) = store().await;
    let mailboxes = MailboxRepository::new(&connection);
    let mut archive = postio_model::Mailbox::new(account.id, "Archive", Some('/'));
    archive.role = MailboxRole::Archive;
    mailboxes.create(&mut archive).await.expect("an archive");
    let mut trash = postio_model::Mailbox::new(account.id, "Trash", Some('/'));
    trash.role = MailboxRole::Trash;
    mailboxes.create(&mut trash).await.expect("a trash");

    let mail = |from: &'static str, subject: &'static str, hours: i64| Mail {
        from,
        subject,
        body: "the budget, attached",
        ago: Duration::hours(hours),
        ..Mail::default()
    };
    let ada = file(&connection, &account, inbox, mail("ada", "Atlas budget", 1)).await;
    let tomas = file(
        &connection,
        &account,
        archive.id,
        mail("tomas", "Harbor plan", 2),
    )
    .await;
    let bo = file(&connection, &account, inbox, mail("bo", "Lunch", 3)).await;
    let binned = file(
        &connection,
        &account,
        trash.id,
        mail("ada", "Old budget", 4),
    )
    .await;

    let labels = LabelRepository::new(&connection);
    let mut atlas = Label::new(account.id, "Atlas");
    let atlas = labels.create(&mut atlas).await.expect("a label");
    let mut harbor = Label::new(account.id, "Harbor");
    let harbor = labels.create(&mut harbor).await.expect("a label");
    labels.attach(ada.id, atlas).await.expect("attach");
    labels.attach(tomas.id, harbor).await.expect("attach");

    World {
        _database: database,
        connection,
        ada: ada.id,
        tomas: tomas.id,
        bo: bo.id,
        binned: binned.id,
    }
}

/// GTK's path: `executor::search`.
async fn searched(connection: &Connection, query: &str) -> BTreeSet<MessageId> {
    let parsed = parse(query, today());
    search(
        connection,
        &SearchRequest {
            account: AccountScope::Unified,
            query: &parsed,
            scope: Scope::AllMail,
            limit: 50,
            order: postio_search::ResultOrder::Newest,
        },
        now(),
    )
    .await
    .expect("search")
    .hits
    .into_iter()
    .map(|hit| hit.message_id)
    .collect()
}

/// The conversation search's, every message its own conversation here.
async fn as_conversations(connection: &Connection, query: &str) -> BTreeSet<MessageId> {
    let results = conversations(connection, query, ConversationOrder::Newest, 0, 50).await;
    assert_eq!(
        results.total,
        results.hits.len() as u64,
        "{query:?}: the total is the hits"
    );
    results
        .hits
        .iter()
        .map(|hit| match hit.key {
            ConversationKey::Lone(id) => id,
            other => panic!("{query:?}: no threads here, got {other:?}"),
        })
        .collect()
}

/// What a way out of no results would say it finds.
async fn counted(connection: &Connection, query: &str) -> u64 {
    postio_index::executor::relaxation_counts(
        connection,
        AccountScope::Unified,
        &[Relaxation {
            loosen: Loosen::Drop { token: 0 },
            query: query.to_owned(),
        }],
        today(),
    )
    .await
    .expect("a count")[0]
}

/// `query` answers `expected` every way it can be asked.
async fn answers(world: &World, query: &str, expected: &[MessageId]) {
    let expected: BTreeSet<MessageId> = expected.iter().copied().collect();
    let connection = &world.connection;
    assert_eq!(
        searched(connection, query).await,
        expected,
        "{query:?}: search"
    );
    assert_eq!(
        as_conversations(connection, query).await,
        expected,
        "{query:?}: conversations"
    );
    assert_eq!(
        counted(connection, query).await,
        expected.len() as u64,
        "{query:?}: counted"
    );
}

#[tokio::test]
async fn a_set_of_people_is_either_and_negated_neither() {
    let world = world().await;
    let (ada, tomas, bo) = (world.ada, world.tomas, world.bo);
    answers(
        &world,
        "from:{ada@example.com tomas@example.com}",
        &[ada, tomas],
    )
    .await;
    answers(&world, "from:{ada tomas}", &[ada, tomas]).await;
    answers(&world, "budget from:{ada tomas}", &[ada, tomas]).await;
    answers(&world, "from:{tomas nobody@example.com}", &[tomas]).await;
    answers(&world, "-from:{ada tomas}", &[bo]).await;
    answers(&world, "budget -from:{ada tomas}", &[bo]).await;
    answers(&world, "from:{ada tomas} -from:ada", &[tomas]).await;
    answers(&world, "from:{nobody@example.com none@example.com}", &[]).await;
}

#[tokio::test]
async fn one_value_in_braces_is_the_plain_clause() {
    let world = world().await;
    for (set, plain) in [
        ("from:{ada}", "from:ada"),
        ("label:{atlas}", "label:atlas"),
        ("-in:{archive}", "-in:archive"),
    ] {
        assert_eq!(
            searched(&world.connection, set).await,
            searched(&world.connection, plain).await,
            "{set:?}"
        );
    }
}

#[tokio::test]
async fn a_set_of_labels_or_folders_is_either_too() {
    let world = world().await;
    let (ada, tomas, bo) = (world.ada, world.tomas, world.bo);
    answers(&world, "label:{atlas harbor}", &[ada, tomas]).await;
    answers(&world, r#"label:{harbor "no such"}"#, &[tomas]).await;
    answers(&world, "-label:{atlas harbor}", &[bo]).await;
    answers(&world, "budget label:{atlas harbor}", &[ada, tomas]).await;
    answers(&world, "in:{inbox archive}", &[ada, tomas, bo]).await;
    answers(&world, "from:{ada tomas} label:atlas", &[ada]).await;
}

#[tokio::test]
async fn a_set_that_names_a_folder_lifts_all_mails_exclusions() {
    // An `in:` names a folder, so the trash it names is searched; a set
    // of them does the same (`names_a_folder`).
    let world = world().await;
    answers(&world, "in:{archive trash}", &[world.tomas, world.binned]).await;
    answers(&world, "-in:{archive inbox}", &[]).await;
}

#[tokio::test]
async fn the_from_facet_of_a_set_lists_both_people() {
    let world = world().await;
    let results = conversations(
        &world.connection,
        "from:{ada tomas}",
        ConversationOrder::BestMatch,
        0,
        10,
    )
    .await;
    let mut senders = Vec::new();
    for count in &results.facets.senders {
        let address = postio_storage::sql::one(
            &world.connection,
            "SELECT address FROM addresses WHERE id = ?1",
            [count.id.get()],
            |row| row.col::<String>(0),
        )
        .await
        .expect("the sender's address");
        senders.push(format!("{address} {}", count.conversations));
    }
    senders.sort();
    assert_eq!(senders, ["ada@example.com 1", "tomas@example.com 1"]);
}
