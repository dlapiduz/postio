//! What moving the bodies out of `search_documents` is worth, measured.
//!
//! #407 asks for the saving to be recorded, and a number arrived at by
//! arithmetic would be worth nothing: the question is how many *pages* SQLite
//! stops carrying, and that depends on the tokenizer, the b-tree fanout and
//! how much of a mail corpus is repeated words. So this builds a store, fills
//! it, and asks `dbstat`.
//!
//! `#[ignore]` because it is a measurement rather than an assertion — it takes
//! seconds, it prints, and what it prints is only meaningful next to the
//! account it was run against. Run it with:
//!
//! ```text
//! cargo test -p postio-index --test body_index_size -- --ignored --nocapture
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
#[ignore = "a measurement, not an assertion; see the module docs"]
fn what_the_bodies_cost_in_each_place() {
    const MESSAGES: usize = 5_000;

    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    ensure_schema(&connection).expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    for n in 0..MESSAGES {
        let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some(format!("Re: engine notes {n}"));
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).expect("create");
        index_body(&connection, message.id.get(), Some(&a_body(n))).expect("index");
    }

    let text: i64 = connection
        .query_row(
            "SELECT sum(length(body)) FROM search_documents",
            [],
            |row| row.get(0),
        )
        .expect("sum");
    let documents = table_bytes(&connection, "search_documents");
    let metadata_index = table_bytes(&connection, "messages_fts");
    let body_index = table_bytes(&connection, "message_bodies_fts");

    let mb = |bytes: i64| bytes as f64 / (1024.0 * 1024.0);
    println!("\n{MESSAGES} messages, {:.1} MB of body text\n", mb(text));
    println!(
        "  search_documents (metadata + the body column)  {:>8.2} MB",
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
        "\n  what dropping search_documents.body saves      {:>8.2} MB",
        mb(text)
    );
    println!(
        "  ... as a share of the three tables above       {:>8.1} %\n",
        100.0 * text as f64 / (documents + metadata_index + body_index) as f64
    );
}
