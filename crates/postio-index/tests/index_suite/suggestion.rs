//! A search that found nothing offers the word that was meant. #1524, ADR 0037.
//!
//! The ranking is unit-tested in `postio-search` with no database at all.
//! What those tests cannot see is whether the vocabulary they rank is
//! actually there: on this engine it is rebuilt from `search_documents`,
//! the table the triggers keep in step with `messages` — and a suggestion
//! drawn from an empty term list is a feature that silently never fires.

use super::executor::at;
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

async fn from(
    connection: &Connection,
    account: &postio_model::Account,
    mailbox: postio_model::MailboxId,
    name: &str,
) {
    let mut message = Message::new(account.id, mailbox, at(9));
    message.from = vec![EmailAddress::new(Some(name), format!("{name}@example.com"))];
    message.subject = Some("Voice lessons".to_owned());
    MessageRepository::new(connection)
        .create(&mut message)
        .await
        .expect("create message");
}

async fn results_for(
    connection: &Connection,
    account: &postio_model::Account,
    typed: &str,
) -> postio_search::SearchResults {
    let query = parse(typed, at(12).date_naive());
    let request = SearchRequest {
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 10,
        order: postio_search::ResultOrder::Relevance,
    };
    search(connection, &request, at(12)).await.expect("search")
}

#[tokio::test]
async fn a_misspelled_name_is_answered_with_the_one_in_the_mailbox() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    for _ in 0..3 {
        from(&connection, &account, mailbox, "hannah").await;
    }

    let results = results_for(&connection, &account, "hanah").await;

    assert_eq!(results.total_hits, 0, "the match itself stays exact");
    let offered = results
        .suggestion
        .expect("a mailbox holding `hannah` should offer it for `hanah`");
    assert_eq!(offered.term, "hannah");
    assert_eq!(
        offered.documents, 3,
        "the offer says what it would find, which is what makes it worth taking"
    );
}

#[tokio::test]
async fn a_query_that_found_something_is_not_second_guessed() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    from(&connection, &account, mailbox, "hannah").await;

    let results = results_for(&connection, &account, "hannah").await;

    assert!(results.total_hits > 0);
    assert_eq!(
        results.suggestion, None,
        "a query that worked is not one to offer alternatives to"
    );
}

#[tokio::test]
async fn a_word_the_mailbox_does_not_resemble_gets_no_offer() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    from(&connection, &account, mailbox, "hannah").await;

    let results = results_for(&connection, &account, "xylophone").await;

    assert_eq!(results.total_hits, 0);
    assert_eq!(results.suggestion, None, "nothing here resembles it");
}

#[tokio::test]
async fn a_quoted_word_is_taken_at_its_word() {
    // Quotes say "this word, exactly": the one way to search for `hanah`
    // itself once the box answers a bare `hanah` with `hannah`.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    from(&connection, &account, mailbox, "hannah").await;

    let results = results_for(&connection, &account, "\"hanah\"").await;

    assert_eq!(results.total_hits, 0);
    assert_eq!(results.suggestion, None);
}

#[tokio::test]
async fn a_query_with_a_filter_is_left_alone() {
    // `from:ada hanah` found nothing, and the filter is at least as likely to
    // be why. Correcting the word would answer a question nobody asked.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    from(&connection, &account, mailbox, "hannah").await;

    let results = results_for(&connection, &account, "from:ada hanah").await;

    assert_eq!(results.total_hits, 0);
    assert_eq!(results.suggestion, None);
}

#[tokio::test]
async fn an_unfinished_address_is_answered_with_the_one_in_the_mailbox() {
    // Terms match whole words, so the first twenty letters of a list's
    // address are a different word from the address -- and the search found
    // nothing for what was, to the person typing it, obviously the list.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    for _ in 0..3 {
        from(&connection, &account, mailbox, "riversidechoirparents").await;
    }

    let results = results_for(&connection, &account, "riversidechoirparent").await;

    assert_eq!(results.total_hits, 0, "the match itself stays exact");
    assert_eq!(
        results
            .suggestion
            .map(|offer| (offer.term, offer.documents)),
        Some(("riversidechoirparents".to_owned(), 3))
    );
}

#[tokio::test]
async fn a_word_only_a_body_holds_is_offered_too() {
    // The old vocabulary was the newest few thousand senders and subjects,
    // so a word that only ever appeared in a message's text was never
    // offered, however common it was.
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let mut message = Message::new(account.id, mailbox, at(9));
    message.subject = Some("Weekly note".to_owned());
    MessageRepository::new(&connection)
        .create(&mut message)
        .await
        .expect("create message");
    postio_index::index::index_body(
        &connection,
        message.id.get(),
        Some("the reimbursement form is attached"),
    )
    .await
    .expect("index body");

    let results = results_for(&connection, &account, "reimburse").await;

    assert_eq!(results.total_hits, 0);
    assert_eq!(
        results.suggestion.map(|offer| offer.term).as_deref(),
        Some("reimbursement")
    );
}

#[tokio::test]
async fn a_zero_hit_search_reads_what_resembles_the_word_not_the_mailbox() {
    // Terms match whole words, so a name typed letter by letter is a
    // zero-hit search at every pause until it is complete (#1613). The
    // offer is looked up through the index, so its cost follows how much
    // mail resembles the word, not how much mail there is.
    use postio_storage::test_support::counting::counted_async;
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    for _ in 0..3 {
        from(&connection, &account, mailbox, "hannah").await;
    }
    for n in 0..120 {
        from(&connection, &account, mailbox, &format!("stranger{n}")).await;
    }

    let cost = counted_async(async || {
        let results = results_for(&connection, &account, "hanah").await;
        assert_eq!(
            results.suggestion.map(|offer| offer.term).as_deref(),
            Some("hannah")
        );
    })
    .await;
    assert!(
        cost.rows < 60,
        "a zero-hit search read {} rows of a 123-message mailbox",
        cost.rows
    );
}
