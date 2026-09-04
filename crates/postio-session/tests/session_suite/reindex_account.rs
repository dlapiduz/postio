//! The account-detail "Rebuild search index" action (#981).
//!
//! The maintainer's own decision on #981 names three properties an
//! implementation has to keep: it is a pure local rebuild, reachable
//! per-account, and it reports progress as it goes rather than running
//! silently. These are the pass's own promises; `postio-index`'s tests
//! already hold the row-level ones (a cleared index is a candidate again,
//! and only for the account that was cleared).

use postio_model::{BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::repository::{AccountRepository, MessageRepository};
use postio_storage::test_support;

fn hits(connection: &rusqlite::Connection, account: postio_model::AccountId, query: &str) -> usize {
    let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
    postio_index::search(
        connection,
        &postio_index::SearchRequest {
            account: postio_model::AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 500,
            order: postio_search::ResultOrder::Relevance,
        },
        chrono::Utc::now(),
    )
    .expect("search runs")
    .total_hits as usize
}

fn second_account(
    connection: &rusqlite::Connection,
) -> (postio_model::Account, postio_model::ids::MailboxId) {
    let mut account = postio_model::Account::new(
        "Second",
        postio_model::EmailAddress::new(None::<String>, "grace@example.org"),
    );
    AccountRepository::new(connection)
        .create(&mut account)
        .expect("second account");
    let mailbox = test_support::mailbox(connection, &account, "INBOX");
    (account, mailbox.id)
}

#[test]
fn a_rebuild_makes_mail_findable_again_and_reports_progress_as_it_goes() {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection);

    let messages = MessageRepository::new(&connection);
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    message.subject = Some("Quarterly engine notes".to_string());
    messages.create(&mut message).expect("create");
    // A real stored body -- not just a role played by `index_body_of` --
    // because `reindex_account`'s own body pass reads it back the ordinary
    // way (`MessageRepository::body`) to write it again, and a message
    // with no stored body re-indexes to nothing.
    messages
        .set_body(
            message.id,
            &postio_storage::repository::StoredBody {
                text: Some("the analytical engine, in full".to_owned()),
                html: None,
                headers: None,
                headers_truncated: false,
                encoding_problems: false,
            },
            BodyState::Full,
        )
        .expect("store a real body");
    postio_index::index::index_body_of(
        &connection,
        message.id.get(),
        &postio_model::MessageBody {
            text: Some("the analytical engine, in full".to_owned()),
            html: None,
        },
    )
    .expect("index the body the ordinary way");

    assert_eq!(
        hits(&connection, account.id, "analytical"),
        1,
        "findable before the rebuild, or this proves nothing about it"
    );
    drop(connection);

    let mut progress: Vec<(u32, u32)> = Vec::new();
    let reindexed = postio_session::reindex_account(&database, account.id, |done, total| {
        progress.push((done, total));
    })
    .expect("the rebuild runs");

    assert_eq!(reindexed, 1, "the one message with local mail to rebuild");
    assert_eq!(
        progress.first(),
        Some(&(0, 1)),
        "the first report names the total before any work is done"
    );
    assert_eq!(
        progress.last(),
        Some(&(1, 1)),
        "the last report says the rebuild actually finished"
    );

    let connection = database.connection().expect("checkout");
    assert_eq!(
        hits(&connection, account.id, "analytical"),
        1,
        "still findable after the rebuild rewrote the row it was found through"
    );
}

#[test]
fn a_rebuild_touches_only_the_account_it_was_asked_for() {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (first, first_inbox) = test_support::account_with_inbox(&connection);
    let (second, second_inbox) = second_account(&connection);

    let messages = MessageRepository::new(&connection);
    for (account, inbox, word) in [
        (first.id, first_inbox, "alpha"),
        (second.id, second_inbox, "beta"),
    ] {
        let mut message = Message::new(account, inbox, chrono::Utc::now());
        messages.create(&mut message).expect("create");
        messages
            .set_body(
                message.id,
                &postio_storage::repository::StoredBody {
                    text: Some(word.to_owned()),
                    html: None,
                    headers: None,
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .expect("store a real body");
        postio_index::index::index_body_of(
            &connection,
            message.id.get(),
            &postio_model::MessageBody {
                text: Some(word.to_owned()),
                html: None,
            },
        )
        .expect("index the body");
    }
    drop(connection);

    let reindexed =
        postio_session::reindex_account(&database, first.id, |_, _| {}).expect("the rebuild runs");
    assert_eq!(reindexed, 1, "only the first account's one message");

    let connection = database.connection().expect("checkout");
    assert_eq!(
        hits(&connection, first.id, "alpha"),
        1,
        "the account that was rebuilt is still findable"
    );
    assert_eq!(
        hits(&connection, second.id, "beta"),
        1,
        "the other account's mail was never touched, so it was never at risk"
    );
    assert!(
        postio_index::index::messages_missing_body_text_for_account(
            &connection,
            second.id.get(),
            10
        )
        .expect("candidates")
        .is_empty(),
        "the second account's index was never cleared, so it has nothing to catch up on"
    );
}

#[test]
fn a_store_with_nothing_local_for_this_account_costs_one_query_and_no_writes() {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");
    let (account, _inbox) = test_support::account_with_inbox(&connection);
    drop(connection);

    let mut progress: Vec<(u32, u32)> = Vec::new();
    let reindexed = postio_session::reindex_account(&database, account.id, |done, total| {
        progress.push((done, total));
    })
    .expect("the rebuild runs");

    assert_eq!(reindexed, 0);
    assert_eq!(
        progress,
        vec![(0, 0)],
        "one report -- nothing to do, and the caller still hears that it finished"
    );
}
