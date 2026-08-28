//! The idle pass that trains a body-compression dictionary (ADR 0020).
//!
//! The hazard this guards is the one #327 and #416 both were: a capability
//! written, tested, and never called. `postio_storage::body::train_dictionary`
//! is worth about a further 28% of the default store, and worth exactly
//! nothing until something on a running Postio invokes it.
//!
//! What the pass owes the rest of the application:
//!
//! * **It declines a store with nothing to learn from.** A fresh account has
//!   no corpus, and a dictionary trained on eleven messages describes eleven
//!   messages.
//! * **It does not train on every start.** Dictionaries are never deleted
//!   while a row names them, so a pass that retrained hourly would leave a
//!   table of near-identical dictionaries nothing may ever remove.
//! * **Bodies written before it ran keep reading.** Nothing is rewritten.

use postio_model::{BodyState, Message};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;

/// Enough mail, and mail-shaped enough, for zstd's trainer to find structure.
const CORPUS: usize = 200;

fn a_body(seed: usize) -> String {
    let mut text = String::new();
    for line in 0..30 {
        text.push_str(&format!(
            "On the {line}th, Ada wrote about invoice {seed}, the quarterly \
             reconciliation, and the printer nobody has fixed.\n"
        ));
    }
    text
}

/// Fills `database` with `count` messages that have bodies.
fn corpus(database: &postio_storage::test_support::TempDatabase, count: usize) {
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);
    for seed in 0..count {
        let mut message = Message::new(
            account.id,
            inbox,
            chrono::Utc::now() - chrono::Duration::minutes(seed as i64),
        );
        message.subject = Some(format!("Invoice {seed}"));
        let id = messages.create(&mut message).expect("create");
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(a_body(seed)),
                    ..StoredBody::default()
                },
                BodyState::Full,
            )
            .expect("store a body");
    }
}

fn dictionaries(database: &postio_storage::test_support::TempDatabase) -> i64 {
    let connection = database.connection().expect("checkout");
    connection
        .query_row("SELECT count(*) FROM body_dictionaries", [], |row| {
            row.get(0)
        })
        .expect("count dictionaries")
}

#[test]
fn a_store_with_no_corpus_trains_nothing() {
    let database = test_support::temp();
    corpus(&database, 3);

    assert!(
        !postio_session::train_body_dictionary(&database).expect("the pass runs"),
        "three messages are not a mailbox to learn from"
    );
    assert_eq!(dictionaries(&database), 0);
}

#[test]
fn a_store_with_a_corpus_trains_once_and_then_leaves_it_alone() {
    let database = test_support::temp();
    corpus(&database, CORPUS);

    assert!(
        postio_session::train_body_dictionary(&database).expect("the pass runs"),
        "a corpus this size is worth a dictionary"
    );
    assert_eq!(dictionaries(&database), 1);

    // The second start. Nothing has grown, so nothing is trained: a pass that
    // retrained every time would leave a table of near-identical dictionaries
    // that nothing may ever delete, because rows name them.
    assert!(
        !postio_session::train_body_dictionary(&database).expect("a second pass"),
        "an unchanged corpus is not worth a second dictionary"
    );
    assert_eq!(dictionaries(&database), 1);
}

#[test]
fn bodies_written_before_the_dictionary_still_read_after_it() {
    // The failure worth being afraid of: a zstd frame can only be read with
    // the dictionary it was written against, so training must not orphan the
    // mail that was already local.
    let database = test_support::temp();
    corpus(&database, CORPUS);

    let connection = database.connection().expect("checkout");
    let messages = MessageRepository::new(&connection);
    let before: Vec<(i64, String)> = connection
        .prepare("SELECT id FROM messages ORDER BY id")
        .expect("prepare")
        .query_map([], |row| row.get::<_, i64>(0))
        .expect("query")
        .map(|id| {
            let id = postio_model::MessageId::new(id.expect("an id"));
            let text = messages
                .body(id)
                .expect("body")
                .expect("the row")
                .text
                .expect("a body");
            (id.get(), text)
        })
        .collect();
    drop(connection);

    assert!(postio_session::train_body_dictionary(&database).expect("the pass runs"));

    let connection = database.connection().expect("checkout");
    let messages = MessageRepository::new(&connection);
    for (id, text) in before {
        assert_eq!(
            messages
                .body(postio_model::MessageId::new(id))
                .expect("body")
                .expect("the row")
                .text
                .as_deref(),
            Some(text.as_str()),
            "message {id} stopped reading when the dictionary arrived"
        );
    }
}

#[test]
fn a_body_written_after_the_pass_uses_the_dictionary_and_reads_back() {
    let database = test_support::temp();
    corpus(&database, CORPUS);
    assert!(postio_session::train_body_dictionary(&database).expect("the pass runs"));

    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);
    let mut message = Message::new(account.id, inbox, chrono::Utc::now());
    let id = messages.create(&mut message).expect("create");
    let text = a_body(9_999);
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(text.clone()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("store a body");

    let named: Option<i64> = connection
        .query_row(
            "SELECT body_dictionary_id FROM messages WHERE id = ?1",
            [id.get()],
            |row| row.get(0),
        )
        .expect("the column");
    assert!(
        named.is_some(),
        "a write after the pass must use the dictionary the pass trained"
    );
    assert_eq!(
        messages
            .body(id)
            .expect("body")
            .expect("the row")
            .text
            .as_deref(),
        Some(text.as_str())
    );
}
