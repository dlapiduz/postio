//! Search chooses an occurrence after applying mailbox and flag constraints.
use chrono::Utc;
use postio_index::executor::facets;
use postio_index::{SearchRequest, index, search};
use postio_model::{AccountScope, ContentIdentity, Flag, Mailbox, MailboxRole, Message};
use postio_search::{ResultOrder, facets::Scope, parse};
use postio_storage::{
    repository::{MailboxRepository, MessageRepository},
    test_support,
};

#[tokio::test]
async fn one_content_hit_uses_a_qualifying_mailbox_occurrence() {
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut archive = Mailbox::new(account.id, "Archive", Some('/'));
    archive.role = MailboxRole::Archive;
    MailboxRepository::new(&connection)
        .create(&mut archive)
        .await
        .unwrap();
    index::ensure_schema(&connection).await.unwrap();
    let repo = MessageRepository::new(&connection);
    let now = Utc::now();
    let mut first = Message::new(account.id, inbox, now);
    first.subject = Some("shared nebula".into());
    first.server.content_identity = Some(ContentIdentity::new("jmap-email", "email-1"));
    repo.create(&mut first).await.unwrap();
    let mut second = first.clone();
    second.mailbox_id = archive.id;
    second.flags.insert(Flag::Seen);
    repo.create(&mut second).await.unwrap();
    for (text, scope, expected) in [
        ("nebula", Scope::AllMail, None),
        ("nebula in:Archive", Scope::AllMail, Some(second.id)),
        ("nebula is:read", Scope::AllMail, Some(second.id)),
        ("nebula is:unread", Scope::AllMail, Some(first.id)),
        ("nebula", Scope::Inbox, Some(first.id)),
    ] {
        let query = parse(text, now.date_naive());
        for order in [ResultOrder::Relevance, ResultOrder::Newest] {
            let results = search(
                &connection,
                &SearchRequest {
                    account: AccountScope::Account(account.id),
                    query: &query,
                    scope,
                    limit: 10,
                    order,
                },
                now,
            )
            .await
            .unwrap();
            assert_eq!(results.total_hits, 1, "{text}, {order:?}");
            assert_eq!(results.hits.len(), 1, "{text}, {order:?}");
            if let Some(id) = expected {
                assert_eq!(results.hits[0].message_id, id);
            }
        }
    }
    let documents: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM search_documents", ())
            .await
            .unwrap();
    assert_eq!(documents, 1);
    let query = parse("nebula", now.date_naive());
    let counts = facets(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: ResultOrder::Relevance,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        counts
            .scopes
            .iter()
            .find(|s| s.scope == Scope::AllMail)
            .unwrap()
            .hits,
        1
    );
    for token in ["is:unread", "in:Archive", "in:INBOX"] {
        assert_eq!(
            counts
                .refinements
                .iter()
                .find(|r| r.token == token)
                .unwrap()
                .hits,
            1,
            "{token}"
        );
    }
    repo.delete(&[first.id]).await.unwrap();
    let query = parse("nebula", now.date_naive());
    let results = search(
        &connection,
        &SearchRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            scope: Scope::AllMail,
            limit: 10,
            order: ResultOrder::Relevance,
        },
        now,
    )
    .await
    .unwrap();
    assert_eq!(
        results.total_hits, 1,
        "expunging one occurrence keeps the document"
    );
    assert_eq!(results.hits[0].message_id, second.id);
}

#[tokio::test]
async fn body_and_header_indexes_are_owned_once_and_repair_invalidates_them() {
    use postio_model::{BodyState, Headers, MessageBody};
    use postio_storage::repository::StoredBody;
    let store = test_support::memory().await;
    let connection = store.connect().await.unwrap();
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    index::ensure_schema(&connection).await.unwrap();
    let repo = MessageRepository::new(&connection);
    let now = Utc::now();
    let mut first = Message::new(account.id, inbox, now);
    first.server.content_identity = Some(ContentIdentity::new("jmap-email", "email-1"));
    repo.create(&mut first).await.unwrap();
    let mut second = first.clone();
    repo.create(&mut second).await.unwrap();
    let body = StoredBody {
        text: Some("uniquequasar".into()),
        html: None,
        headers: Some("X-Mailer: shared-fixture\r\n".into()),
        headers_truncated: false,
        encoding_problems: false,
    };
    repo.set_body(first.id, &body, BodyState::Full)
        .await
        .unwrap();
    assert_eq!(
        index::messages_missing_body_text_for_account(&connection, account.id.get(), 10)
            .await
            .unwrap()
            .len(),
        1
    );
    index::index_body_of(
        &connection,
        second.id.get(),
        &MessageBody {
            text: body.text.clone(),
            html: None,
        },
    )
    .await
    .unwrap();
    index::index_headers(
        &connection,
        second.id.get(),
        &Headers::from_iter([("X-Mailer", "shared-fixture")]),
    )
    .await
    .unwrap();
    repo.delete(&[first.id]).await.unwrap();
    for text in ["uniquequasar", "header:x-mailer=shared-fixture"] {
        let query = parse(text, now.date_naive());
        let results = search(
            &connection,
            &SearchRequest {
                account: AccountScope::Account(account.id),
                query: &query,
                scope: Scope::AllMail,
                limit: 10,
                order: ResultOrder::Relevance,
            },
            now,
        )
        .await
        .unwrap();
        assert_eq!(results.total_hits, 1, "{text}");
        assert_eq!(results.hits[0].message_id, second.id);
    }
    for table in ["message_search_bodies", "message_headers"] {
        let count: i64 =
            postio_storage::sql::scalar(&connection, &format!("SELECT count(*) FROM {table}"), ())
                .await
                .unwrap();
        assert_eq!(count, 1, "{table}");
    }
    // A parser repair can change decoded text/header interpretation while
    // the backend's underlying byte identity remains immutable.
    repo.set_body(
        second.id,
        &StoredBody {
            text: Some("repaired words".into()),
            headers: None,
            ..body
        },
        BodyState::Full,
    )
    .await
    .unwrap();
    assert_eq!(
        index::messages_missing_body_text_for_account(&connection, account.id.get(), 10)
            .await
            .unwrap(),
        vec![second.id.get()]
    );
    let headers: i64 =
        postio_storage::sql::scalar(&connection, "SELECT count(*) FROM message_headers", ())
            .await
            .unwrap();
    assert_eq!(headers, 0);
}
