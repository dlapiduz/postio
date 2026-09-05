//! What storing the header block costs, measured against the text beside it.
//!
//! ADR 0025 stores every message's whole header block so the indexing policy
//! stays revisable without re-downloading a mailbox, and #884 asks what that
//! is worth in bytes before anyone has to find out on their own disk. The
//! guess in the issue is that it could go either way: the median body is 325
//! bytes (ADR 0020) and a `Received` chain alone is routinely a kilobyte, so
//! the block may well be *larger* than the body it belongs to — while also
//! being the most compressible text in a mailbox, because every message from
//! one provider carries the same boilerplate, the same field names and the
//! same DKIM shapes. Arithmetic cannot settle that. This builds a store,
//! fills it from the corpus, trains the dictionary and asks `dbstat`.
//!
//! `#[ignore]` because it is a measurement rather than an assertion — the same
//! reason `postio-index`'s `body_index_size.rs` is, whose shape this follows.
//! Run it with:
//!
//! ```text
//! cargo test -p postio-storage --test header_block_size -- --ignored --nocapture
//! ```
//!
//! # What it said, and how far to trust it
//!
//! On the corpus, 8,200 stored messages:
//!
//! ```text
//! header block, uncompressed: 4,642,000 bytes   body text: 2,285,200
//! header block, stored:       2,879,600 bytes   body text: 1,655,000
//! header compression:         1.61x             text:      1.38x
//! blocks as a share of both:  63.5%
//! ```
//!
//! So the issue's open question resolves: **the blocks are larger than the
//! bodies**, by roughly two to one, and they do compress better — but at
//! 1.61x they are nowhere near ADR 0020's 2.19x for bodies against a trained
//! dictionary.
//!
//! Read the ratio as a ceiling rather than a forecast. This repeats the same
//! few dozen fixtures, so identical blocks recur in a way real mail does not:
//! a real mailbox repeats *boilerplate* between messages from one provider,
//! not whole blocks. The share — headers being most of what the two axes cost
//! together — is the number to carry forward, and it is the argument for the
//! 256 KiB cap being a real bound rather than a formality.
//!
//! **It prints sizes and never content.** Header values carry `Received`
//! chains with addresses and internal hostnames, and this repository's logs
//! and output carry ids, counts and outcomes only.

use postio_model::BodyState;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use rusqlite::Connection;

/// Real page usage for one column's b-tree, the way `body_index_size.rs`
/// measures: `dbstat` rather than `length()`, because what matters is the
/// pages SQLite actually carries.
fn table_bytes(connection: &Connection, name: &str) -> i64 {
    connection
        .query_row(
            "SELECT coalesce(sum(pgsize), 0) FROM dbstat WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .expect("dbstat")
}

/// The uncompressed length of one column across every row.
fn stored_bytes(connection: &Connection, column: &str) -> i64 {
    connection
        .query_row(
            &format!("SELECT coalesce(sum(length({column})), 0) FROM messages"),
            [],
            |row| row.get(0),
        )
        .expect("sum")
}

#[test]
#[ignore = "a measurement, not an assertion; see the module docs"]
fn what_the_header_blocks_cost_next_to_the_text() {
    // The corpus, repeated: real header blocks from real mail, which is the
    // whole point — a generated block would have exactly the repetition the
    // dictionary is being asked about, and would answer its own question.
    const ROUNDS: usize = 200;

    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let (account, mailbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    let mut raw_block_bytes = 0usize;
    let mut raw_text_bytes = 0usize;
    let mut stored_count = 0usize;

    for round in 0..ROUNDS {
        for fixture in postio_model::test_corpus::all() {
            let parsed = postio_model::mime::parse(fixture.bytes());
            let Some(block) = postio_model::headers::block_of(fixture.bytes()) else {
                continue;
            };
            let text = parsed.body.text.clone();

            let mut message = postio_model::Message::new(account.id, mailbox, chrono::Utc::now());
            message.subject = Some(format!("fixture {} round {round}", fixture.name()));
            let id = messages.create(&mut message).expect("create");

            raw_block_bytes += block.text.len();
            raw_text_bytes += text.as_deref().map_or(0, str::len);
            stored_count += 1;

            messages
                .set_body(
                    id,
                    &StoredBody {
                        text,
                        html: parsed.body.html.clone(),
                        headers: Some(block.text),
                        headers_truncated: block.truncated,
                        encoding_problems: false,
                    },
                    BodyState::Full,
                )
                .expect("set");
        }

        // Train once the corpus is big enough, so the rows after it are
        // written against a dictionary exactly as a real store's are. ADR 0017
        // trains once and then only on tenfold growth, so most of a real
        // mailbox is written against one.
        if round == 0 {
            let _ = postio_storage::body::train_dictionary(&connection);
        }
    }

    let messages_pages = table_bytes(&connection, "messages");
    let compressed_headers = stored_bytes(&connection, "body_headers");
    let compressed_text = stored_bytes(&connection, "body_text");

    println!("\nmessages stored:            {stored_count}");
    println!("header block, uncompressed: {raw_block_bytes} bytes");
    println!("header block, stored:       {compressed_headers} bytes");
    println!("body text, uncompressed:    {raw_text_bytes} bytes");
    println!("body text, stored:          {compressed_text} bytes");
    if compressed_headers > 0 {
        println!(
            "header compression:         {:.2}x",
            raw_block_bytes as f64 / compressed_headers as f64
        );
    }
    if compressed_text > 0 {
        println!(
            "text compression:           {:.2}x",
            raw_text_bytes as f64 / compressed_text as f64
        );
    }
    println!(
        "blocks as a share of both:  {:.1}%",
        100.0 * compressed_headers as f64 / (compressed_headers + compressed_text).max(1) as f64
    );
    println!("messages table pages:       {messages_pages} bytes\n");
}
