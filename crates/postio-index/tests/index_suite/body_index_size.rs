//! What the two indexes cost next to the text they are built from, measured.
//!
//! #407 asked for the saving from moving bodies out of `search_documents` to
//! be recorded, and a number arrived at by arithmetic would have been worth
//! nothing: the question is how many *pages* SQLite stops carrying, and that
//! depends on the tokenizer, the b-tree fanout and how much of a mail corpus
//! is repeated words. So this builds a store, fills it, and asks `dbstat`.
//!
//! That saving has since been taken — `b4a54bfe` dropped
//! `search_documents.body` and the bodies left SQLite — and this measured a
//! `sum(length(body))` over that column until #1466. It went on doing so for
//! two changes, broken, because it was `#[ignore]`d for being slow and
//! nothing anywhere ran the ignored tests; the tier #1450 built is what
//! finally executed it, and it failed on the first run.
//!
//! What is left is the live half, and it is the half worth keeping: what the
//! metadata index and the body index each cost against the corpus that
//! produced them. The corpus is measured at its source — the strings
//! `a_body` builds — rather than from a column, which is both what the
//! schema now permits and the better question: it compares an index against
//! its input rather than against a second copy of it.
//!
//! POSTIO-MEASUREMENT: its output is numbers a person reads, so it runs on
//! the nightly timer rather than the merge path. `.config/nextest.toml`'s
//! `profile.default` filter is what holds it back; `--profile nightly` runs it.
//!
//! It is a measurement rather than an assertion — it takes
//! seconds, it prints, and what it prints is only meaningful next to the
//! account it was run against. Run it with:
//!
//! ```text
//! cargo nextest run --profile nightly -p postio-index -E 'test(/^body_index_size::/)'
//! ```

use postio_index::index::{ensure_schema, index_body};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use rusqlite::Connection;

/// Mail-shaped text: a few hundred words with the repetition real mail has —
/// a quoted parent, a signature, the same handful of names.
fn a_body(n: usize) -> String {
    let quoted = "> On Monday the analytical engine was mentioned again, and the \
                  question of whether the mill can be made to fold a card back \
                  into the store came up for the fourth time this quarter.\n";
    let mut text = String::new();
    text.push_str(&format!(
        "Thanks for the note about item {n}. The difference engine's seventh \
         column is finished and the drawings are with the printer.\n\n"
    ));
    for _ in 0..6 {
        text.push_str(quoted);
    }
    text.push_str(
        "\n--\nAda Lovelace\nAnalytical Engine Programme\nada@example.com\n\
         This message and any attachments are intended for the addressee.\n",
    );
    text
}

fn table_bytes(connection: &Connection, name: &str) -> i64 {
    // `dbstat` reports real page usage per b-tree, including the shadow
    // tables an FTS5 index is made of -- which is the only honest way to
    // compare a virtual table with an ordinary column.
    connection
        .query_row(
            "SELECT coalesce(sum(pgsize), 0) FROM dbstat
              WHERE name = ?1 OR name LIKE ?1 || '\\_%' ESCAPE '\\'",
            [name],
            |row| row.get(0),
        )
        .expect("dbstat")
}

#[test]
fn what_the_bodies_cost_in_each_place() {
    const MESSAGES: usize = 5_000;

    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut text: i64 = 0;
    for n in 0..MESSAGES {
        let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some(format!("Re: engine notes {n}"));
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).expect("create");
        let body = a_body(n);
        text += body.len() as i64;
        index_body(&connection, message.id.get(), Some(&body)).expect("index");
    }
    let documents = table_bytes(&connection, "search_documents");
    let metadata_index = table_bytes(&connection, "messages_fts");
    let body_index = table_bytes(&connection, "message_bodies_fts");

    let mb = |bytes: i64| bytes as f64 / (1024.0 * 1024.0);
    println!("\n{MESSAGES} messages, {:.1} MB of body text\n", mb(text));
    println!(
        "  search_documents (the metadata)                {:>8.2} MB",
        mb(documents)
    );
    println!(
        "  messages_fts     (the metadata index)          {:>8.2} MB",
        mb(metadata_index)
    );
    println!(
        "  message_bodies_fts (the body index)            {:>8.2} MB",
        mb(body_index)
    );
    println!(
        "\n  the body text these were built from           {:>8.2} MB",
        mb(text)
    );
    println!(
        "  ... what the three tables above cost of it     {:>8.1} %\n",
        100.0 * (documents + metadata_index + body_index) as f64 / text as f64
    );
}
