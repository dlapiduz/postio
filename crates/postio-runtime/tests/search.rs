//! Proves `postio-index`'s executor is reachable from `postio-runtime`
//! through an ordinary manifest dependency.
//!
//! Before `postio-svx`, `postio_search`'s executor sat behind an `index`
//! Cargo feature that nothing in the workspace could turn on without pulling
//! `rusqlite` into `postio-gtk`'s graph (Cargo resolves features as a union
//! across the whole workspace resolve), so `search` had never run inside
//! Postio. `postio-index` is the split that fixes it: this test does not
//! exercise the executor's *behaviour* — `postio-index`'s own tests already
//! do that thoroughly — it exists only to fail loudly if the seam from this
//! crate ever closes again.

use chrono::Utc;
use postio_index::{SearchRequest, index, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

#[test]
fn the_executor_is_reachable_from_postio_runtimes_own_dependency_graph() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    index::ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);

    let mut message = Message::new(account.id, mailbox, Utc::now());
    message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
    message.subject = Some("Quarterly report".to_string());
    MessageRepository::new(&connection)
        .create(&mut message)
        .expect("create message");

    let query = parse("quarterly", Utc::now().date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
    };
    let results = search(&connection, &request, Utc::now()).expect("search");

    assert_eq!(results.hits.len(), 1);
    assert_eq!(results.hits[0].message_id, message.id);
}
