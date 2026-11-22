use chrono::{TimeZone, Utc};
use postio_model::{EmailAddress, Message};
use postio_storage::Connection;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use std::time::Instant;

const ACCOUNTS: u64 = 4;
const PER_ACCOUNT: u64 = 30_000;
const UNCOMMON_WORD: &str = "quarterly";
const COMMON_WORD: &str = "regarding";
const SENDER_COUNT: u64 = 500;
const LIMIT: u32 = 50;

struct Xorshift64(u64);
impl Xorshift64 {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, bound: u64) -> u64 {
        self.next() % bound
    }
}

#[tokio::main]
async fn main() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64(0x5eed_1234_5678_9abc);
    let repository = MessageRepository::new(&connection);

    let mut account_ids = Vec::new();
    connection.execute_batch("BEGIN").await.expect("begin");
    for a in 0..ACCOUNTS {
        let (account, mailbox) = test_support::account_with_inbox(&connection).await;
        account_ids.push(account.id.get());
        for i in 0..PER_ACCOUNT {
            let sender = rng.below(SENDER_COUNT);
            let received_at = base + chrono::Duration::minutes((i * ACCOUNTS + a) as i64);
            let mut message = Message::new(account.id, mailbox, received_at);
            message.from = vec![EmailAddress::new(
                Some(format!("Sender {sender}")),
                format!("sender{sender}@example.com"),
            )];
            message.subject = Some(format!("Weekly update {i}"));
            message.size = 1024 + rng.below(4096);
            repository.create(&mut message).await.expect("create");

            let mut body = format!("{COMMON_WORD} the status as of message {i}");
            if i % 100 == 0 {
                body.push_str(&format!(" {UNCOMMON_WORD} figures attached"));
            }
            postio_index::index::index_body(&connection, message.id.get(), Some(&body))
                .await
                .expect("index");
        }
    }
    connection.execute_batch("COMMIT").await.expect("commit");
    connection.execute_batch("ANALYZE").await.expect("analyze");
    println!(
        "corpus: {ACCOUNTS} accounts x {PER_ACCOUNT} = {} messages\n",
        ACCOUNTS * PER_ACCOUNT
    );

    let correlated = "(EXISTS (SELECT 1 FROM messages_fts WHERE rowid = m.id AND messages_fts MATCH ?1)
   OR EXISTS (SELECT 1 FROM message_bodies_fts WHERE rowid = m.id AND message_bodies_fts MATCH ?1))";

    for (label, word) in [("uncommon", UNCOMMON_WORD), ("common", COMMON_WORD)] {
        println!("=========== {label} word: {word:?} ===========");

        let a_sql = format!(
            "SELECT m.id FROM messages m
              WHERE m.account_id = ?2 AND {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report(
            "A  account-scoped (today)",
            &connection,
            &a_sql,
            &[
                turso::Value::Text(word.to_string()),
                turso::Value::Integer(account_ids[0]),
            ],
        )
        .await;

        let b_sql = format!(
            "SELECT m.id FROM messages m
              WHERE {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report(
            "B  unified, no index",
            &connection,
            &b_sql,
            &[turso::Value::Text(word.to_string())],
        )
        .await;

        let arms: Vec<String> = account_ids
            .iter()
            .map(|id| {
                format!(
                    "SELECT * FROM (SELECT m.id AS id, m.received_at AS received_at
                       FROM messages m
                      WHERE m.account_id = {id} AND {correlated}
                      ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT})"
                )
            })
            .collect();
        let d_sql = format!(
            "SELECT id FROM ({}) ORDER BY received_at DESC, id DESC LIMIT {LIMIT}",
            arms.join(" UNION ALL ")
        );
        report(
            "D  per-account UNION ALL",
            &connection,
            &d_sql,
            &[turso::Value::Text(word.to_string())],
        )
        .await;
    }

    println!("\n>>> adding idx_messages_recency ON messages (received_at DESC, id DESC)");
    let built = Instant::now();
    connection
        .execute_batch("CREATE INDEX idx_messages_recency ON messages (received_at DESC, id DESC)")
        .await
        .expect("create index");
    println!("    built in {:?}", built.elapsed());
    connection.execute_batch("ANALYZE").await.expect("analyze");

    for (label, word) in [("uncommon", UNCOMMON_WORD), ("common", COMMON_WORD)] {
        println!("=========== {label} word, with the new index ===========");
        let c_sql = format!(
            "SELECT m.id FROM messages m
              WHERE {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report(
            "C  unified + recency index",
            &connection,
            &c_sql,
            &[turso::Value::Text(word.to_string())],
        )
        .await;

        let a_sql = format!(
            "SELECT m.id FROM messages m
              WHERE m.account_id = ?2 AND {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report(
            "A' account-scoped, index present",
            &connection,
            &a_sql,
            &[
                turso::Value::Text(word.to_string()),
                turso::Value::Integer(account_ids[0]),
            ],
        )
        .await;
    }

    let hits_join = "FROM (
             SELECT rowid AS rid, bm25(messages_fts) AS meta, NULL AS body
               FROM messages_fts WHERE messages_fts MATCH ?1
             UNION ALL
             SELECT rowid, NULL, bm25(message_bodies_fts)
               FROM message_bodies_fts WHERE message_bodies_fts MATCH ?1
          ) hits CROSS JOIN messages m ON m.id = hits.rid";
    println!("\n=========== Form::Driven (ranked by relevance) ===========");
    for (label, word) in [("uncommon", UNCOMMON_WORD), ("common", COMMON_WORD)] {
        let scoped = format!(
            "SELECT m.id {hits_join} WHERE m.account_id = ?2
              GROUP BY m.id ORDER BY min(coalesce(hits.meta, hits.body)) LIMIT {LIMIT}"
        );
        report(
            &format!("E  {label}: driven, account-scoped"),
            &connection,
            &scoped,
            &[
                turso::Value::Text(word.to_string()),
                turso::Value::Integer(account_ids[0]),
            ],
        )
        .await;
        let unified = format!(
            "SELECT m.id {hits_join}
              GROUP BY m.id ORDER BY min(coalesce(hits.meta, hits.body)) LIMIT {LIMIT}"
        );
        report(
            &format!("F  {label}: driven, unified"),
            &connection,
            &unified,
            &[turso::Value::Text(word.to_string())],
        )
        .await;
    }

    // Measured in both orders: the first of the pair pays for a cold page cache.
    println!("\n=========== write cost ===========");
    measure_insert(&connection, "1st: with idx_messages_recency").await;
    connection
        .execute_batch("DROP INDEX idx_messages_recency")
        .await
        .expect("drop");
    measure_insert(&connection, "2nd: without it").await;
    connection
        .execute_batch("CREATE INDEX idx_messages_recency ON messages (received_at DESC, id DESC)")
        .await
        .expect("recreate");
    measure_insert(&connection, "3rd: with it again").await;

    let pages = postio_storage::sql::scalar(&connection, "PRAGMA page_count", ())
        .await
        .unwrap_or(0);
    let page_size = postio_storage::sql::scalar(&connection, "PRAGMA page_size", ())
        .await
        .unwrap_or(0);
    println!(
        "the store: {} KiB for {} messages",
        pages * page_size / 1024,
        ACCOUNTS * PER_ACCOUNT
    );
}

async fn report(label: &str, connection: &Connection, sql: &str, params: &[turso::Value]) {
    let plan: Vec<String> = postio_storage::sql::all(
        connection,
        &format!("EXPLAIN QUERY PLAN {sql}"),
        params.to_vec(),
        |row| postio_storage::sql::RowExt::col::<String>(row, 3),
    )
    .await
    .expect("plan rows");

    let run = async |connection: &Connection| -> usize {
        postio_storage::sql::all(connection, sql, params.to_vec(), |row| {
            postio_storage::sql::RowExt::col::<i64>(row, 0)
        })
        .await
        .expect("run")
        .len()
    };
    let _ = run(connection).await;
    let mut best = std::time::Duration::MAX;
    let mut rows = 0;
    for _ in 0..5 {
        let started = Instant::now();
        rows = run(connection).await;
        best = best.min(started.elapsed());
    }
    println!("{label:34} {best:>12.2?}  ({rows} rows)");
    for line in plan {
        println!("{:34} | {line}", "");
    }
}

async fn measure_insert(connection: &Connection, label: &str) {
    let base = Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap();
    let account = postio_storage::sql::scalar(connection, "SELECT id FROM accounts LIMIT 1", ())
        .await
        .expect("an account");
    let mailbox = postio_storage::sql::scalar(connection, "SELECT id FROM mailboxes LIMIT 1", ())
        .await
        .expect("a mailbox");

    let started = Instant::now();
    connection.execute_batch("BEGIN").await.expect("begin");
    for i in 0..20_000i64 {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, sort_at, size,
                                       body_state)
                 VALUES (?1, ?2, ?3, ?3, 1024, 'headers_only')",
                (
                    account,
                    mailbox,
                    (base + chrono::Duration::minutes(i)).timestamp_millis(),
                ),
            )
            .await
            .expect("insert");
    }
    connection.execute_batch("COMMIT").await.expect("commit");
    println!("20,000 inserts {label:26} {:>12.2?}", started.elapsed());
    connection
        .execute_batch("DELETE FROM messages WHERE received_at >= 1893456000000")
        .await
        .expect("cleanup");
}
