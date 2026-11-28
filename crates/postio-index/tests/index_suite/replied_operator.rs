use std::collections::{BTreeSet, HashSet};

use chrono::Duration;
use postio_index::executor::{ConversationRequest, search_conversations};
use postio_index::{SearchRequest, search};
use postio_model::{AccountScope, MessageId, ThreadId};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_search::results::{ConversationKey, ConversationOrder};
use postio_storage::test_support::counting::{Counts, counted_async, install};

use crate::conversations::{Mail, file, now, store, thread, today};

struct World {
    _database: postio_storage::Store,
    connection: postio_storage::Checkout,
    asked: MessageId,
    answered: MessageId,
    lone_note: MessageId,
    unanswered: MessageId,
    flagged_elsewhere: MessageId,
    plan: ThreadId,
    offsite: ThreadId,
    invoice: ThreadId,
}

async fn world() -> World {
    let (database, connection, account, inbox) = store().await;
    let mail = |from: &'static str, subject: &'static str, days: i64, answered: bool| Mail {
        from,
        subject,
        body: "the plan for the quarter",
        ago: Duration::days(days),
        answered,
        ..Mail::default()
    };
    let asked = file(
        &connection,
        &account,
        inbox,
        mail("ada", "Quarter plan", 9, false),
    )
    .await;
    let answered = file(
        &connection,
        &account,
        inbox,
        mail("test", "Re: Quarter plan", 8, false),
    )
    .await;
    let plan = thread(&connection, &account, &[asked.id, answered.id]).await;
    let unanswered = file(
        &connection,
        &account,
        inbox,
        mail("grace", "Offsite", 7, false),
    )
    .await;
    let offsite = thread(&connection, &account, &[unanswered.id]).await;
    let flagged_elsewhere = file(
        &connection,
        &account,
        inbox,
        mail("tomas", "Invoice", 6, true),
    )
    .await;
    let invoice = thread(&connection, &account, &[flagged_elsewhere.id]).await;
    let lone_note = file(
        &connection,
        &account,
        inbox,
        mail("test", "Notes to self", 5, false),
    )
    .await;
    World {
        _database: database,
        connection,
        asked: asked.id,
        answered: answered.id,
        lone_note: lone_note.id,
        unanswered: unanswered.id,
        flagged_elsewhere: flagged_elsewhere.id,
        plan,
        offsite,
        invoice,
    }
}

async fn messages(world: &World, text: &str) -> BTreeSet<MessageId> {
    let query = parse(text, today());
    let request = SearchRequest {
        mailboxes: None,
        account: AccountScope::Unified,
        query: &query,
        scope: Scope::AllMail,
        limit: 50,
        order: postio_search::ResultOrder::Newest,
    };
    search(&world.connection, &request, now())
        .await
        .expect("a message search")
        .hits
        .into_iter()
        .map(|hit| hit.message_id)
        .collect()
}

async fn conversations(world: &World, text: &str) -> HashSet<ConversationKey> {
    let query = parse(text, today());
    let request = ConversationRequest {
        account: AccountScope::Unified,
        query: &query,
        order: ConversationOrder::Newest,
        offset: 0,
        limit: 50,
        today: today(),
    };
    search_conversations(&world.connection, &request, now())
        .await
        .expect("a conversation search")
        .hits
        .into_iter()
        .map(|hit| hit.key)
        .collect()
}

#[tokio::test]
async fn is_replied_finds_every_message_of_a_thread_the_person_wrote_in() {
    let world = world().await;
    assert_eq!(
        messages(&world, "is:replied").await,
        [world.asked, world.answered, world.lone_note].into(),
        "the question, the person's answer in its thread, and a note the person sent alone"
    );
    assert_eq!(
        messages(&world, "-is:replied").await,
        [world.unanswered, world.flagged_elsewhere].into(),
        "a server's \\Answered with no reply in the store is not a reply the person can see"
    );
}

#[tokio::test]
async fn the_conversation_search_finds_the_same_threads() {
    let world = world().await;
    let replied: HashSet<_> = [
        ConversationKey::Thread(world.plan),
        ConversationKey::Lone(world.lone_note),
    ]
    .into();
    assert_eq!(conversations(&world, "is:replied").await, replied);
    assert_eq!(
        conversations(&world, "is:replied quarter").await,
        replied,
        "composes with words"
    );
    assert_eq!(
        conversations(&world, "-is:replied").await,
        [
            ConversationKey::Thread(world.offsite),
            ConversationKey::Thread(world.invoice)
        ]
        .into()
    );
}

#[tokio::test]
async fn is_replied_costs_the_same_statements_however_many_threads_were_answered() {
    let mut costs = Vec::new();
    for threads in [3_usize, 30] {
        let (_database, connection, account, inbox) = store().await;
        for nth in 0..threads {
            let days = 2 * nth as i64 + 2;
            let asked = file(
                &connection,
                &account,
                inbox,
                Mail {
                    from: "ada",
                    subject: "Question",
                    ago: Duration::days(days),
                    ..Mail::default()
                },
            )
            .await;
            let reply = file(
                &connection,
                &account,
                inbox,
                Mail {
                    from: "test",
                    subject: "Re: Question",
                    ago: Duration::days(days - 1),
                    ..Mail::default()
                },
            )
            .await;
            thread(&connection, &account, &[asked.id, reply.id]).await;
        }
        install(&connection);
        let query = parse("is:replied", today());
        let request = ConversationRequest {
            account: AccountScope::Unified,
            query: &query,
            order: ConversationOrder::Newest,
            offset: 0,
            limit: 25,
            today: today(),
        };
        let mut total = 0;
        let counts: Counts = counted_async(async || {
            total = search_conversations(&connection, &request, now())
                .await
                .expect("a conversation search")
                .total;
        })
        .await;
        assert_eq!(total, threads as u64, "every answered thread is one hit");
        costs.push(counts.statements);
    }
    assert_eq!(
        costs[0], costs[1],
        "statements over 3 answered threads, then 30: a read per answered thread would show here"
    );
}
