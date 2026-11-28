use chrono::{TimeZone, Utc};
use postio_index::executor::facets;
use postio_index::{SearchRequest, index, search};
use postio_model::{AccountScope, EmailAddress, Message};
use postio_search::{ResultOrder, facets::Scope, parse};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting;

fn is_a_driven_scan(step: &str) -> bool {
    matches!(
        step,
        "SCAN hits" | "SCAN CONSTANT ROW" | "SCAN capped AS c" | "SCAN matched"
    ) || step.starts_with("SCAN (subquery-")
}

#[tokio::test]
async fn a_word_with_a_date_filter_seeks_each_hit_by_its_content() {
    let store = test_support::memory().await;
    let connection = store.connect().await.expect("checkout");
    index::ensure_schema(&connection).await.expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);
    for (nth, month) in [1, 3, 5, 7].into_iter().enumerate() {
        let received = Utc.with_ymd_and_hms(2025, month, 10, 9, 0, 0).unwrap();
        let mut message = Message::new(account.id, inbox, received);
        message.from = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly figures {nth}"));
        messages.create(&mut message).await.expect("create message");
        index::index_body(&connection, message.id.get(), Some("the quarterly report"))
            .await
            .expect("index body");
    }
    let today = chrono::NaiveDate::from_ymd_opt(2025, 9, 1).unwrap();
    let now = Utc.with_ymd_and_hms(2025, 9, 1, 12, 0, 0).unwrap();

    for text in [
        "quarterly after:2025-04-01",
        "quarterly before:2025-04-01",
        "quarterly after:2025-02-01 before:2025-06-01",
    ] {
        let query = parse(text, today);
        for order in [ResultOrder::Relevance, ResultOrder::Newest] {
            let request = SearchRequest {
                account: AccountScope::Account(account.id),
                query: &query,
                scope: Scope::AllMail,
                limit: 25,
                order,
            };
            counting::record();
            let results = search(&connection, &request, now).await.expect("search");
            facets(&connection, &request).await.expect("facets");
            let statements = counting::recorded();
            assert!(
                !results.hits.is_empty(),
                "{text}: the fixture is meant to answer this"
            );

            let driven: Vec<&String> = statements
                .keys()
                .filter(|sql| sql.contains("hits.rid"))
                .collect();
            for sql in &driven {
                let steps = counting::plan_steps(&connection, sql)
                    .await
                    .unwrap_or_else(|error| panic!("cannot plan {sql}: {error}"));
                let walks: Vec<String> = counting::scans(&connection, sql)
                    .await
                    .into_iter()
                    .filter(|step| !is_a_driven_scan(step))
                    .collect();
                assert!(
                    walks.is_empty(),
                    "{text}, {order:?}: a statement driven by the hits scans \
                     {walks:?}.\n{sql}\n{steps:#?}"
                );
                let lookups: Vec<&String> = steps
                    .iter()
                    .filter(|step| step.starts_with("SEARCH m "))
                    .collect();
                assert!(
                    !lookups.is_empty()
                        && lookups.iter().all(|step| step.as_str()
                            == "SEARCH m USING INDEX idx_messages_content (content_id=?)"),
                    "{text}, {order:?}: each hit must be looked up by its content, \
                     `idx_messages_content (content_id=?)`; the plan seeks it \
                     with {lookups:?} -- a range walked once per hit (#1809).\n\
                     {sql}\n{steps:#?}"
                );
                let siblings: Vec<&String> = steps
                    .iter()
                    .filter(|step| step.starts_with("SEARCH earlier "))
                    .collect();
                assert!(
                    siblings.iter().all(|step| step.as_str()
                        == "SEARCH earlier USING INDEX idx_messages_content \
                            (content_id=? AND id<?)"),
                    "{text}, {order:?}: the sibling check must seek the content's \
                     other memberships, not a range: {siblings:?}\n{sql}"
                );
                let copies: Vec<&String> = steps
                    .iter()
                    .filter(|step| step.starts_with("SEARCH copy "))
                    .collect();
                assert!(
                    !copies.is_empty()
                        && copies.iter().all(|step| step.as_str()
                            == "SEARCH copy USING INDEX idx_messages_rfc_message_id \
                                (account_id=? AND rfc_message_id=?)"),
                    "{text}, {order:?}: the copy check must seek the message's \
                     copies by its Message-ID: {copies:?}\n{sql}\n{steps:#?}"
                );
            }
            for start in ["SELECT count(*)", "SELECT m.id", "WITH capped"] {
                assert!(
                    driven.iter().any(|sql| sql.trim_start().starts_with(start)),
                    "{text}, {order:?}: no recorded statement joining the hits \
                     starts `{start}`, so this gate cannot see that statement's \
                     plan. Recorded: {:#?}",
                    statements.keys().collect::<Vec<_>>()
                );
            }
        }
    }
}

fn walks_a_growing_table(step: &str) -> bool {
    let Some(table) = step
        .strip_prefix("SCAN ")
        .and_then(|rest| rest.split_whitespace().next())
    else {
        return false;
    };
    counting::GROWING_TABLES.contains(&table)
        || ["attachment_passages", "attachment_extraction", "markers"].contains(&table)
}

fn is_a_keyed_lookup(step: &str) -> bool {
    matches!(
        step,
        "SEARCH m USING INDEX idx_messages_content (content_id=?)"
            | "SEARCH m USING COVERING INDEX idx_messages_content (content_id=?)"
            | "SEARCH m USING INTEGER PRIMARY KEY (rowid=?)"
            | "SEARCH earlier USING INDEX idx_messages_content (content_id=? AND id<?)"
    )
}

#[tokio::test]
async fn a_conversation_search_with_a_date_filter_seeks_each_hit_by_its_content() {
    use crate::conversations::{
        Mail, budget_sheet, file, mail_with_budget_sheet, now, store, today,
    };
    use chrono::Duration;
    use postio_index::executor::{
        ConversationRequest, completions, files, relaxation_counts, search_conversations,
    };
    use postio_search::results::ConversationOrder;

    let (_database, connection, account, inbox) = store().await;
    for (nth, days) in [10, 60, 120, 200].into_iter().enumerate() {
        file(
            &connection,
            &account,
            inbox,
            Mail {
                subject: [
                    "quarterly figures",
                    "quarterly plan",
                    "quarterly review",
                    "quarterly close",
                ][nth],
                body: "the quarterly report",
                file: Some("quarterly.pdf"),
                ago: Duration::days(days),
                ..Mail::default()
            },
        )
        .await;
    }
    let (_, sheet) = mail_with_budget_sheet(&connection, &account, inbox).await;
    postio_index::index::index_attachment_text(&connection, sheet, &budget_sheet())
        .await
        .expect("index the sheet");

    let scopes = [AccountScope::Account(account.id), AccountScope::Unified];
    for text in [
        "quarterly after:2026-06-01",
        "quarterly before:2026-06-01",
        "quarterly after:2026-03-01 before:2026-08-01",
        "kestrel after:2026-06-01",
        "quarterly from:ada after:2026-06-01",
        "quarterly -figures before:2026-08-01",
    ] {
        let query = parse(text, today());
        let relaxations = postio_search::relax::relax(&query);
        for scope in scopes {
            for order in [ConversationOrder::BestMatch, ConversationOrder::Newest] {
                let request = ConversationRequest {
                    account: scope,
                    query: &query,
                    order,
                    offset: 0,
                    limit: 25,
                    today: today(),
                };
                counting::record();
                let results = search_conversations(&connection, &request, now())
                    .await
                    .expect("search");
                let searched = counting::recorded();
                for sql in searched.keys() {
                    let steps = counting::plan_steps(&connection, sql)
                        .await
                        .unwrap_or_else(|error| panic!("cannot plan {sql}: {error}"));
                    let walks: Vec<&String> = steps
                        .iter()
                        .filter(|step| walks_a_growing_table(step))
                        .collect();
                    assert!(
                        walks.is_empty(),
                        "{text}, {scope:?}, {order:?}: a statement of the search \
                         scans {walks:?}.\n{sql}\n{steps:#?}"
                    );
                }
                counting::record();
                files(&connection, &request).await.expect("files");
                relaxation_counts(&connection, scope, &relaxations, today())
                    .await
                    .expect("relaxations");
                completions(&connection, scope, "quar", None)
                    .await
                    .expect("completions");
                let mut statements = counting::recorded();
                statements.extend(searched);
                assert!(
                    results.total > 0,
                    "{text}: the fixture is meant to answer this"
                );

                let keyed: Vec<&String> = statements
                    .keys()
                    .filter(|sql| joins_by_content(sql))
                    .collect();
                for sql in &keyed {
                    let steps = counting::plan_steps(&connection, sql)
                        .await
                        .unwrap_or_else(|error| panic!("cannot plan {sql}: {error}"));
                    let lookups: Vec<&String> = steps
                        .iter()
                        .filter(|step| {
                            step.starts_with("SEARCH m ") || step.starts_with("SEARCH earlier ")
                        })
                        .collect();
                    let walks_of_messages = flat(sql).matches("FROM messages m WHERE").count();
                    let ranged = lookups
                        .iter()
                        .filter(|step| !is_a_keyed_lookup(step))
                        .count();
                    assert!(
                        ranged <= walks_of_messages,
                        "{text}, {scope:?}, {order:?}: each hit must be looked up by \
                         its content or its id; the plan seeks it with {lookups:?} -- \
                         a range walked once per hit (#1809).\n{sql}\n{steps:#?}"
                    );
                    let walks: Vec<&String> = steps
                        .iter()
                        .filter(|step| walks_a_growing_table(step))
                        .collect();
                    assert!(
                        walks.is_empty(),
                        "{text}, {scope:?}, {order:?}: a statement keyed by content \
                         scans {walks:?}.\n{sql}\n{steps:#?}"
                    );
                }
                let (call, marker) = if text.contains("from:") {
                    (
                        "an index's arm on the set path",
                        "ON m.content_id = h.content_id",
                    )
                } else {
                    ("the walk of the match", "ON m.content_id = hits.rid")
                };
                assert!(
                    keyed
                        .iter()
                        .any(|sql| flat(sql).contains(marker) && flat(sql).contains("SELECT m.id")),
                    "{text}, {scope:?}, {order:?}: {call} was not recorded, so this gate \
                     cannot see its plan. Recorded: {:#?}",
                    statements.keys().collect::<Vec<_>>()
                );
                if text.starts_with("kestrel") {
                    assert!(
                        keyed
                            .iter()
                            .any(|sql| sql.contains("ON m.content_id = p.content_id")),
                        "{text}: the Files tab's passages were not recorded"
                    );
                }
            }
        }
    }
}

fn joins_by_content(sql: &str) -> bool {
    flat(sql).contains("ON m.content_id = ")
}

fn flat(sql: &str) -> String {
    sql.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[tokio::test]
async fn the_walk_reads_its_people_labels_and_files_from_the_row() {
    use crate::conversations::{Mail, file, now, store, today};
    use chrono::Duration;
    use postio_index::executor::{ConversationRequest, search_conversations};
    use postio_search::results::ConversationOrder;

    let (_database, connection, account, inbox) = store().await;
    for (nth, days) in [10, 60].into_iter().enumerate() {
        file(
            &connection,
            &account,
            inbox,
            Mail {
                subject: ["quarterly figures", "quarterly plan"][nth],
                body: "the quarterly report",
                file: Some("quarterly.pdf"),
                ago: Duration::days(days),
                ..Mail::default()
            },
        )
        .await;
    }
    file(
        &connection,
        &account,
        inbox,
        Mail {
            from: "test",
            subject: "Re: quarterly figures",
            ago: Duration::days(9),
            ..Mail::default()
        },
    )
    .await;

    for text in [
        "quarterly",
        "has:attachment",
        "quarterly from:ada",
        "from:ada",
        "is:replied",
        "quarterly is:replied",
    ] {
        let query = parse(text, today());
        let request = ConversationRequest {
            account: AccountScope::Account(account.id),
            query: &query,
            order: ConversationOrder::BestMatch,
            offset: 0,
            limit: 25,
            today: today(),
        };
        counting::record();
        let results = search_conversations(&connection, &request, now())
            .await
            .expect("search");
        assert!(
            results.total > 0,
            "{text}: the fixture is meant to answer this"
        );
        let walks: Vec<String> = counting::recorded()
            .into_keys()
            .filter(|sql| flat(sql).contains("SELECT m.id, m.thread_id, m.mailbox_id"))
            .collect();
        assert!(
            !walks.is_empty(),
            "{text}: the walk was not recorded, so this gate cannot see its plan"
        );
        for sql in &walks {
            let steps = counting::plan_steps(&connection, sql)
                .await
                .unwrap_or_else(|error| panic!("cannot plan {sql}: {error}"));
            let lookups: Vec<&String> = steps
                .iter()
                .filter(|step| {
                    ["recipients", "message_labels", "attachments"]
                        .iter()
                        .any(|table| step.contains(table))
                })
                .collect();
            assert!(
                lookups.is_empty(),
                "{text}: the walk looks each row's people, labels or files up \
                 ({lookups:?}) where the row's own columns hold them.\n{sql}\n{steps:#?}"
            );
        }
    }
}
