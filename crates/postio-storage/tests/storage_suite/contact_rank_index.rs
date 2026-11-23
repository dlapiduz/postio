use postio_storage::Connection;
use postio_storage::bind;

const CONTACTS: usize = 20_000;

async fn migrated() -> (postio_storage::Store, postio_storage::Checkout) {
    let store = postio_storage::test_support::memory().await;
    let connection = store.connect().await.expect("a connection");
    (store, connection)
}

async fn definition(connection: &Connection, index: &str) -> String {
    postio_storage::sql::one(
        connection,
        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
        bind![index],
        |row| postio_storage::sql::RowExt::col::<String>(row, 0),
    )
    .await
    .unwrap_or_else(|error| panic!("no index named {index}: {error}"))
}

async fn plan(connection: &Connection, query: &str) -> String {
    postio_storage::test_support::plan(connection, query).await
}

async fn fill(connection: &Connection) {
    connection
        .execute_batch(&format!(
            "INSERT INTO contacts
                 (name, sort_key, name_key, times_seen, last_seen_at, source, state,
                  created_at, updated_at)
             WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i + 1 FROM n WHERE i < {CONTACTS})
             SELECT 'Person ' || i, 'person ' || i, 'person ' || i, i % 50, 1700000000 + i,
                    CASE WHEN i % 3 = 0 THEN 'user' ELSE 'mail' END,
                    CASE WHEN i % 10 = 0 THEN 'deleted' ELSE 'live' END, 0, 0
               FROM n;"
        ))
        .await
        .expect("fill the address book");
}

/// Copied from `ContactRepository::complete`: the SQL the planner sees is what this test is about,
/// and a copy that must be kept in step makes a change visible.
const SEARCH: &str = "SELECT id FROM contacts WHERE state = 'live' \
     ORDER BY CASE WHEN source = 'mail' THEN 1 ELSE 0 END, \
              last_seen_at DESC, times_seen DESC, id \
     LIMIT 20";

#[tokio::test]
async fn autocomplete_is_answered_from_the_index_rather_than_by_sorting_everyone() {
    let (_store, connection) = migrated().await;
    fill(&connection).await;

    let plan = plan(&connection, SEARCH).await;
    assert!(
        !postio_storage::test_support::sorts(&plan),
        "contact autocomplete sorts the whole address book on every \
         keystroke. `LIMIT 20` cannot help: the sort has to see every row \
         before it knows which twenty come first.\n  plan: {plan}\n  index: {}",
        definition(&connection, "idx_contacts_rank").await
    );
    assert!(
        plan.contains("idx_contacts_rank"),
        "the sort is gone but not because of this index, which is the one \
         that is supposed to serve it.\n  plan: {plan}"
    );
}

#[tokio::test]
async fn the_index_leads_with_the_band_that_the_ordering_leads_with() {
    let (_store, connection) = migrated().await;
    let sql = definition(&connection, "idx_contacts_rank").await;
    assert!(
        sql.find("state")
            .is_some_and(|at| at < sql.find("source").unwrap()),
        "live people are a range of the index, ahead of the band: {sql}"
    );
    assert!(
        sql.contains("source") && sql.contains("last_seen_at") && sql.contains("times_seen"),
        "the index has to name every term of the ordering it serves: {sql}"
    );
    assert!(
        sql.find("source").unwrap() < sql.find("last_seen_at").unwrap(),
        "the band leads the ordering, so it has to lead the index -- an index \
         that starts at `last_seen_at` cannot serve an ORDER BY that starts \
         somewhere else: {sql}"
    );
    assert!(
        sql.find("last_seen_at").unwrap() < sql.find("times_seen").unwrap(),
        "recency leads frequency since #424, and #430 corrected the ADR to \
         say so: {sql}"
    );
}

#[tokio::test]
async fn the_rows_still_come_back_in_the_order_the_product_promises() {
    // Asserts the answer, not the plan: an index the planner uses can still return the wrong rows.
    let (_store, connection) = migrated().await;
    fill(&connection).await;

    let mut statement = connection.prepare(SEARCH).await.expect("prepare");
    let ids: Vec<i64> = postio_storage::sql::mapped(&mut statement, (), |row| {
        postio_storage::sql::RowExt::col(row, 0)
    })
    .await
    .expect("query");

    assert_eq!(ids.len(), 20);
    let source_of = async |id: i64| -> String {
        postio_storage::sql::one(
            &connection,
            "SELECT source FROM contacts WHERE id = ?1",
            bind![id],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a contact")
    };
    let last_seen_of = async |id: i64| -> i64 {
        postio_storage::sql::one(
            &connection,
            "SELECT last_seen_at FROM contacts WHERE id = ?1",
            bind![id],
            |row| postio_storage::sql::RowExt::col(row, 0),
        )
        .await
        .expect("a contact")
    };

    let mut sources = Vec::new();
    let mut times: Vec<i64> = Vec::new();
    for id in &ids {
        sources.push(source_of(*id).await);
        times.push(last_seen_of(*id).await);
    }
    assert!(
        ids.iter().all(|id| id % 10 != 0),
        "every tenth person is deleted, and none of them may be offered"
    );
    assert!(
        sources.iter().all(|source| source == "user"),
        "every one of the first twenty should be a contact the user created: \
         the harvested band sorts after it however recent it is"
    );
    let mut descending = times.clone();
    descending.sort_by(|a, b| b.cmp(a));
    assert_eq!(
        times, descending,
        "within a band, the most recent comes first"
    );
}
