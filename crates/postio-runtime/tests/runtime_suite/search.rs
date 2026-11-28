//! Fails if `postio-index`'s executor stops being reachable from this crate through an ordinary
//! manifest dependency.

use chrono::Utc;
use postio_index::{SearchRequest, index, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

#[tokio::test]
async fn the_executor_is_reachable_from_postio_runtimes_own_dependency_graph() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let mut message = Message::new(account.id, mailbox, Utc::now());
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.subject = Some("Quarterly report".to_string());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create message");

    let query = parse("quarterly", Utc::now().date_naive());
    let request = SearchRequest {
        mailboxes: None,
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    let results = search(&connection, &request, Utc::now())
        .await
        .expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, message.id);
}
