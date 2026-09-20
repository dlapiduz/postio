//! What the two indexes cost next to the text they are built from, measured.
//!
//! #407 asked for the saving from moving bodies out of `search_documents` to
//! be recorded, and a number arrived at by arithmetic would have been worth
//! nothing: the question is how many *pages* SQLite stops carrying, and that
//! depends on the tokenizer, the b-tree fanout and how much of a mail corpus
//! is repeated words. So this builds a store, fills it, and weighs it.
//!
//! That saving has since been taken — `b4a54bfe` dropped
//! `search_documents.body` and the bodies left SQLite — and this measured a
//! `sum(length(body))` over that column until #1466. It went on doing so for
//! two changes, broken, because it was `#[ignore]`d for being slow and
//! nothing anywhere ran the ignored tests; the tier #1450 built is what
//! finally executed it, and it failed on the first run.
//!
//! What is left is the live half, and it is the half worth keeping: what the
//! metadata and the body index cost against the corpus that produced them.
//! The corpus is measured at its source — the strings `a_body` builds —
//! rather than from a column, which is both what the schema now permits and
//! the better question: it compares an index against its input rather than
//! against a second copy of it.
//!
//! **Weighed on disk, because this engine has no `dbstat`.** There is no
//! per-b-tree accounting to ask for, so the store is measured as a file
//! between steps and each figure is a delta. Coarser than `dbstat` in that it
//! cannot separate a table from its index; truer in that it counts everything
//! the step adds to the file, which is the number that reaches a disk.
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

/// What the store weighs on disk, right now.
///
/// The log is folded back into the file first, or this reads a database whose
/// newest pages are still in `postio.db-wal`.
async fn store_bytes(store: &postio_storage::test_support::TempStore) -> i64 {
    store.truncate_log().await.expect("fold the log back in");
    std::fs::metadata(store.directory().join("postio.db"))
        .expect("the store is on disk")
        .len() as i64
}

#[tokio::test]
async fn what_the_bodies_cost_in_each_place() {
    const MESSAGES: usize = 5_000;

    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let empty = store_bytes(&database).await;

    let mut text: i64 = 0;
    let mut ids = Vec::with_capacity(MESSAGES);
    for n in 0..MESSAGES {
        let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some(format!("Re: engine notes {n}"));
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).await.expect("create");
        text += a_body(n).len() as i64;
        ids.push(message.id.get());
    }
    // The mail and its metadata index, with no body text anywhere yet.
    let metadata = store_bytes(&database).await - empty;

    for (n, id) in ids.iter().enumerate() {
        index_body(&connection, *id, Some(&a_body(n)))
            .await
            .expect("index");
    }
    let bodies = store_bytes(&database).await - empty - metadata;

    let mb = |bytes: i64| bytes as f64 / (1024.0 * 1024.0);
    println!("\n{MESSAGES} messages, {:.1} MB of body text\n", mb(text));
    println!(
        "  the mail and its metadata index add            {:>8.2} MB",
        mb(metadata)
    );
    println!(
        "  the body text and its index add                {:>8.2} MB",
        mb(bodies)
    );
    println!(
        "\n  the body text these were built from           {:>8.2} MB",
        mb(text)
    );
    println!(
        "  ... what the whole store costs of it           {:>8.1} %\n",
        100.0 * (metadata + bodies) as f64 / text as f64
    );
}
