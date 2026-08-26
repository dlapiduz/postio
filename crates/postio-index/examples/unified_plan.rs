//! The measurement behind ADR 0005 Q5a: what unified search costs (#435).
//!
//! `postio-index`'s recency path walks `idx_messages_account_list`
//! (`account_id, received_at DESC, id DESC`) in the order the caller asked
//! for and stops at `LIMIT`. A composite index can only supply that ordering
//! while its leading column is pinned to one value, so dropping the
//! `account_id` predicate — which is all "unified" means — takes the ordering
//! away with it. This runs the four candidate plans against a corpus that can
//! actually tell them apart, and prints `EXPLAIN QUERY PLAN` beside each time.
//!
//! It lives here rather than in `benches/` because it answers a question that
//! was asked once. `search_budget.rs` is the standing budget gate, and it is
//! single-account: it cannot see any of this, which is the other half of what
//! #435 found.
//!
//! ```sh
//! cargo run --release -p postio-index --example unified_plan
//! ```
//!
//! `--release` is not optional — a debug build measures rustc, not SQLite.
//! The corpus is a fixed-seed xorshift like `search_budget.rs`'s, so a run is
//! reproducible; the four accounts are interleaved in time so that a unified
//! recency ordering genuinely has to merge them rather than concatenate.

use chrono::{TimeZone, Utc};
use postio_model::{EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;
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

fn main() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    postio_index::index::ensure_schema(&connection).expect("schema");

    let base = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let mut rng = Xorshift64(0x5eed_1234_5678_9abc);
    let repository = MessageRepository::new(&connection);

    let mut account_ids = Vec::new();
    connection.execute_batch("BEGIN").expect("begin");
    for a in 0..ACCOUNTS {
        let (account, mailbox) = test_support::account_with_inbox(&connection);
        account_ids.push(account.id.get());
        for i in 0..PER_ACCOUNT {
            let sender = rng.below(SENDER_COUNT);
            // Interleaved in time across accounts, which is what a unified
            // recency ordering actually has to merge.
            let received_at = base + chrono::Duration::minutes((i * ACCOUNTS + a) as i64);
            let mut message = Message::new(account.id, mailbox, received_at);
            message.from = vec![EmailAddress::new(
                Some(format!("Sender {sender}")),
                format!("sender{sender}@example.com"),
            )];
            message.subject = Some(format!("Weekly update {i}"));
            message.size = 1024 + rng.below(4096);
            repository.create(&mut message).expect("create");

            let mut body = format!("{COMMON_WORD} the status as of message {i}");
            if i % 100 == 0 {
                body.push_str(&format!(" {UNCOMMON_WORD} figures attached"));
            }
            postio_index::index::index_body(&connection, message.id.get(), Some(&body))
                .expect("index");
        }
    }
    connection.execute_batch("COMMIT").expect("commit");
    connection.execute_batch("ANALYZE").expect("analyze");
    println!(
        "corpus: {ACCOUNTS} accounts x {PER_ACCOUNT} = {} messages\n",
        ACCOUNTS * PER_ACCOUNT
    );

    let correlated = "(EXISTS (SELECT 1 FROM messages_fts WHERE rowid = m.id AND messages_fts MATCH ?1)
   OR EXISTS (SELECT 1 FROM message_bodies_fts WHERE rowid = m.id AND message_bodies_fts MATCH ?1))";

    for (label, word) in [("uncommon", UNCOMMON_WORD), ("common", COMMON_WORD)] {
        println!("=========== {label} word: {word:?} ===========");

        // A — today's plan, one account named.
        let a_sql = format!(
            "SELECT m.id FROM messages m
              WHERE m.account_id = ?2 AND {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report(
            "A  account-scoped (today)",
            &connection,
            &a_sql,
            &[&word, &account_ids[0]],
        );

        // B — unified, no account predicate, no new index.
        let b_sql = format!(
            "SELECT m.id FROM messages m
              WHERE {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report("B  unified, no index", &connection, &b_sql, &[&word]);

        // D — per-account UNION ALL, each on its own index, merged.
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
        report("D  per-account UNION ALL", &connection, &d_sql, &[&word]);
    }

    // C — unified with the index option 1 proposes.
    println!("\n>>> adding idx_messages_recency ON messages (received_at DESC, id DESC)");
    let built = Instant::now();
    connection
        .execute_batch("CREATE INDEX idx_messages_recency ON messages (received_at DESC, id DESC)")
        .expect("create index");
    println!("    built in {:?}", built.elapsed());
    connection.execute_batch("ANALYZE").expect("analyze");

    for (label, word) in [("uncommon", UNCOMMON_WORD), ("common", COMMON_WORD)] {
        println!("=========== {label} word, with the new index ===========");
        let c_sql = format!(
            "SELECT m.id FROM messages m
              WHERE {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report("C  unified + recency index", &connection, &c_sql, &[&word]);

        let a_sql = format!(
            "SELECT m.id FROM messages m
              WHERE m.account_id = ?2 AND {correlated}
              ORDER BY m.received_at DESC, m.id DESC LIMIT {LIMIT}"
        );
        report(
            "A' account-scoped, index present",
            &connection,
            &a_sql,
            &[&word, &account_ids[0]],
        );
    }

    // The other half of the executor: `Form::Driven`, which orders by bm25
    // rather than by received_at. Its ORDER BY does not depend on
    // idx_messages_account_list at all, so the account predicate should only
    // change selectivity here, not the plan.
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
            &[&word, &account_ids[0]],
        );
        let unified = format!(
            "SELECT m.id {hits_join}
              GROUP BY m.id ORDER BY min(coalesce(hits.meta, hits.body)) LIMIT {LIMIT}"
        );
        report(
            &format!("F  {label}: driven, unified"),
            &connection,
            &unified,
            &[&word],
        );
    }

    // What the index costs on the write path, which is the objection to it.
    // Measured in both orders, because the first run of the pair pays for a
    // cold page cache and the second does not -- taking one order alone had
    // the index looking *faster*, which it is not.
    println!("\n=========== write cost ===========");
    measure_insert(&connection, "1st: with idx_messages_recency");
    connection
        .execute_batch("DROP INDEX idx_messages_recency")
        .expect("drop");
    measure_insert(&connection, "2nd: without it");
    connection
        .execute_batch("CREATE INDEX idx_messages_recency ON messages (received_at DESC, id DESC)")
        .expect("recreate");
    measure_insert(&connection, "3rd: with it again");

    let pages: i64 = connection
        .query_row(
            "SELECT count(*) FROM dbstat WHERE name = 'idx_messages_recency'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(-1);
    let page_size: i64 = connection
        .query_row("PRAGMA page_size", [], |r| r.get(0))
        .unwrap_or(0);
    println!(
        "idx_messages_recency: {pages} pages x {page_size} B = {} KiB for {} messages",
        pages * page_size / 1024,
        ACCOUNTS * PER_ACCOUNT
    );
}

fn report(label: &str, connection: &Connection, sql: &str, params: &[&dyn rusqlite::ToSql]) {
    let plan: Vec<String> = {
        let mut statement = connection
            .prepare(&format!("EXPLAIN QUERY PLAN {sql}"))
            .expect("prepare plan");
        let rows = statement
            .query_map(params, |row| row.get::<_, String>(3))
            .expect("plan rows");
        rows.filter_map(Result::ok).collect()
    };

    let mut statement = connection.prepare(sql).expect("prepare");
    let run = |statement: &mut rusqlite::Statement<'_>| -> usize {
        let rows = statement
            .query_map(params, |row| row.get::<_, i64>(0))
            .expect("run");
        rows.filter_map(Result::ok).count()
    };
    // Warm, then take the best of five: the interesting quantity is the
    // plan's cost, not the page cache's mood.
    let _ = run(&mut statement);
    let mut best = std::time::Duration::MAX;
    let mut rows = 0;
    for _ in 0..5 {
        let started = Instant::now();
        rows = run(&mut statement);
        best = best.min(started.elapsed());
    }
    println!("{label:34} {best:>12.2?}  ({rows} rows)");
    for line in plan {
        println!("{:34} | {line}", "");
    }
}

fn measure_insert(connection: &Connection, label: &str) {
    let base = Utc.with_ymd_and_hms(2030, 1, 1, 0, 0, 0).unwrap();
    let account: i64 = connection
        .query_row("SELECT id FROM accounts LIMIT 1", [], |r| r.get(0))
        .expect("an account");
    let mailbox: i64 = connection
        .query_row("SELECT id FROM mailboxes LIMIT 1", [], |r| r.get(0))
        .expect("a mailbox");

    let started = Instant::now();
    connection.execute_batch("BEGIN").expect("begin");
    for i in 0..20_000i64 {
        connection
            .execute(
                "INSERT INTO messages (account_id, mailbox_id, received_at, size, body_state)
                 VALUES (?1, ?2, ?3, 1024, 'headers_only')",
                rusqlite::params![
                    account,
                    mailbox,
                    (base + chrono::Duration::minutes(i)).timestamp_millis()
                ],
            )
            .expect("insert");
    }
    connection.execute_batch("COMMIT").expect("commit");
    println!("20,000 inserts {label:26} {:>12.2?}", started.elapsed());
    connection
        .execute_batch("DELETE FROM messages WHERE received_at >= 1893456000000")
        .expect("cleanup");
}
