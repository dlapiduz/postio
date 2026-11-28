use postio_model::{AccountScope, BodyState, Message};
use postio_search::facets::Scope;
use postio_storage::Connection;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

const MESSAGES: usize = 260;

fn a_block(nth: usize) -> String {
    format!(
        "Received: from hop-{nth}.example.com by mx.example.net\r\n\
         From: ada@example.com\r\n\
         Subject: Engine notes {nth}\r\n\
         X-Mailer: Mutt 1.5.24 (2015-08-30)"
    )
}

async fn hits(connection: &Connection, account: postio_model::AccountId, query: &str) -> usize {
    let parsed = postio_search::parse(query, chrono::Utc::now().date_naive());
    postio_index::search(
        connection,
        &postio_index::SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(account),
            query: &parsed,
            scope: Scope::AllMail,
            limit: 500,
            order: postio_search::ResultOrder::Relevance,
        },
        chrono::Utc::now(),
    )
    .await
    .expect("search runs")
    .total_hits as usize
}

#[tokio::test]
async fn mail_that_was_already_here_becomes_findable_by_header_and_stays_swept() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let messages = MessageRepository::new(&connection);
    for nth in 0..MESSAGES {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(nth as i64),
        );
        message.subject = Some(format!("Engine notes {nth}"));
        messages.create(&mut message).await.expect("create");
        messages
            .set_body(
                message.id,
                &StoredBody {
                    text: Some("the body".to_string()),
                    html: None,
                    headers: Some(a_block(nth)),
                    headers_truncated: false,
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .await
            .expect("store a block");
    }

    assert_eq!(
        hits(&connection, account.id, "header:x-mailer=mutt").await,
        0,
        "a stored block nothing has indexed is not yet a hit"
    );
    drop(connection);

    let indexed = postio_session::index_local_headers(&database)
        .await
        .expect("the pass runs");
    assert_eq!(indexed, MESSAGES, "every block was visited exactly once");

    let connection = database.connect().await.expect("checkout");
    assert_eq!(
        hits(&connection, account.id, "header:x-mailer=mutt").await,
        MESSAGES,
        "and now every one of them answers the operator the pass exists for"
    );
    assert_eq!(
        hits(&connection, account.id, "header:received=hop-7.example.com").await,
        1,
        "including a value that is unique to one message"
    );
    assert!(
        postio_index::index::messages_missing_header_rows(&connection, 10)
            .await
            .expect("candidates")
            .is_empty(),
        "a swept store leaves no candidates, or the next start sweeps it again"
    );

    drop(connection);
    let second = postio_session::index_local_headers(&database)
        .await
        .expect("the second pass");
    assert_eq!(second, 0, "a caught-up store costs one query and no writes");
}

#[tokio::test]
async fn a_store_whose_blocks_were_never_written_gives_the_pass_nothing_to_do() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;

    let messages = MessageRepository::new(&connection);
    for nth in 0..8 {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(nth),
        );
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).await.expect("create");
    }
    drop(connection);

    assert_eq!(
        postio_session::index_local_headers(&database)
            .await
            .expect("the pass runs"),
        0,
        "no stored block, nothing this pass can do without a network"
    );
}
