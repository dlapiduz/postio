//! Message bodies live in `messages` rows, compressed (ADR 0020).
//!
//! The bytes used to be files in the blob store and the row held their keys.
//! They are columns now, zstd-compressed per value against a dictionary stored
//! in a sibling table, and the compression is entirely below
//! [`MessageRepository`] — nothing above storage knows it happened.
//!
//! What these tests hold down is the part that can silently rot: a value must
//! come back *exactly*, whatever dictionary it was written against, including
//! one that is no longer the current dictionary.

use postio_model::{BodyState, MailboxId, Message, MessageId};
use postio_storage::body;
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use rusqlite::Connection;

fn at(seconds: i64) -> chrono::DateTime<chrono::Utc> {
    chrono::TimeZone::timestamp_opt(&chrono::Utc, 1_770_000_000 + seconds, 0)
        .single()
        .unwrap()
}

fn a_message(
    connection: &Connection,
    mailbox: MailboxId,
    account: postio_model::AccountId,
) -> MessageId {
    let mut message = Message::new(account, mailbox, at(0));
    message.subject = Some("Invoice 42".to_owned());
    MessageRepository::new(connection)
        .create(&mut message)
        .expect("create")
}

/// Mail-shaped text, long enough that compression is actually exercised.
fn a_body(seed: usize) -> String {
    let mut text = String::new();
    for line in 0..40 {
        text.push_str(&format!(
            "On Tuesday the {line}th, Ada wrote about invoice {seed} and the \
             quarterly reconciliation that nobody has finished reading yet.\n"
        ));
    }
    text
}

#[test]
fn a_body_round_trips_through_the_row() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let id = a_message(&connection, inbox, account.id);
    let messages = MessageRepository::new(&connection);

    assert_eq!(
        messages.body(id).expect("body").expect("the row"),
        StoredBody::default(),
        "nothing has been downloaded yet"
    );

    let body = StoredBody {
        text: Some(a_body(1)),
        html: Some(format!("<p>{}</p>", a_body(2))),
        headers: Some("Subject: Invoice 42\r\nFrom: ada@example.com\r\n".to_owned()),
    };
    messages.set_body(id, &body, BodyState::Full).expect("set");

    assert_eq!(messages.body(id).expect("body"), Some(body));
    let stored = messages.get(id).expect("get").expect("the message");
    assert_eq!(stored.sync.body_state, BodyState::Full);
}

#[test]
fn the_bytes_in_the_column_are_compressed_and_not_the_plain_text() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let id = a_message(&connection, inbox, account.id);

    let text = a_body(3);
    let body = StoredBody {
        text: Some(text.clone()),
        ..StoredBody::default()
    };
    MessageRepository::new(&connection)
        .set_body(id, &body, BodyState::Full)
        .expect("set");

    let stored: Vec<u8> = connection
        .query_row(
            "SELECT body_text FROM messages WHERE id = ?1",
            [id.get()],
            |row| row.get(0),
        )
        .expect("the column");
    assert!(
        stored.len() < text.len(),
        "a {} byte body stored as {} bytes is not compressed",
        text.len(),
        stored.len()
    );
    assert!(
        !stored.windows(7).any(|window| window == b"invoice"),
        "the plain text is still readable in the column"
    );
}

#[test]
fn an_empty_or_absent_part_stays_absent() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let id = a_message(&connection, inbox, account.id);
    let messages = MessageRepository::new(&connection);

    let body = StoredBody {
        text: Some(String::new()),
        html: None,
        headers: None,
    };
    messages.set_body(id, &body, BodyState::Full).expect("set");

    let read = messages.body(id).expect("body").expect("the row");
    assert_eq!(
        read.text.as_deref(),
        Some(""),
        "an empty part is empty, not missing"
    );
    assert_eq!(read.html, None);
    assert_eq!(read.headers, None);
}

#[test]
fn a_body_written_against_a_dictionary_round_trips() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    // A corpus to train on.
    let mut ids = Vec::new();
    for seed in 0..64 {
        let id = a_message(&connection, inbox, account.id);
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(a_body(seed)),
                    ..StoredBody::default()
                },
                BodyState::Full,
            )
            .expect("set");
        ids.push(id);
    }

    let dictionary = body::train_dictionary(&connection)
        .expect("train")
        .expect("a corpus this size trains");

    // Everything written before the dictionary existed still reads.
    for (seed, id) in ids.iter().enumerate() {
        let read = messages.body(*id).expect("body").expect("the row");
        assert_eq!(read.text.as_deref(), Some(a_body(seed).as_str()));
    }

    // And a new write uses it.
    let id = a_message(&connection, inbox, account.id);
    let text = a_body(999);
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(text.clone()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("set");

    let named: Option<i64> = connection
        .query_row(
            "SELECT body_dictionary_id FROM messages WHERE id = ?1",
            [id.get()],
            |row| row.get(0),
        )
        .expect("the column");
    assert_eq!(
        named,
        Some(dictionary.get()),
        "a new write names the dictionary"
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

#[test]
fn a_body_written_against_an_older_dictionary_still_reads() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    for seed in 0..64 {
        let id = a_message(&connection, inbox, account.id);
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(a_body(seed)),
                    ..StoredBody::default()
                },
                BodyState::Full,
            )
            .expect("set");
    }
    let first = body::train_dictionary(&connection)
        .expect("train")
        .expect("a dictionary");

    let old = a_message(&connection, inbox, account.id);
    let old_text = a_body(1001);
    messages
        .set_body(
            old,
            &StoredBody {
                text: Some(old_text.clone()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("set");

    // A second training pass supersedes the first.
    for seed in 100..200 {
        let id = a_message(&connection, inbox, account.id);
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(a_body(seed)),
                    ..StoredBody::default()
                },
                BodyState::Full,
            )
            .expect("set");
    }
    let second = body::train_dictionary(&connection)
        .expect("train")
        .expect("a second dictionary");
    assert_ne!(
        first.get(),
        second.get(),
        "training again makes a new dictionary"
    );

    assert_eq!(
        messages
            .body(old)
            .expect("body")
            .expect("the row")
            .text
            .as_deref(),
        Some(old_text.as_str()),
        "a body written against the superseded dictionary must still read"
    );
}

#[test]
fn a_dictionary_a_row_still_names_cannot_be_dropped() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    for seed in 0..64 {
        let id = a_message(&connection, inbox, account.id);
        messages
            .set_body(
                id,
                &StoredBody {
                    text: Some(a_body(seed)),
                    ..StoredBody::default()
                },
                BodyState::Full,
            )
            .expect("set");
    }
    let dictionary = body::train_dictionary(&connection)
        .expect("train")
        .expect("a dictionary");

    let id = a_message(&connection, inbox, account.id);
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(a_body(7)),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("set");

    connection
        .execute(
            "DELETE FROM body_dictionaries WHERE id = ?1",
            [dictionary.get()],
        )
        .expect_err("a dictionary a row names is not droppable: it would take the mail with it");
}

#[test]
fn training_needs_a_corpus() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let messages = MessageRepository::new(&connection);

    assert_eq!(
        body::train_dictionary(&connection).expect("train"),
        None,
        "nothing to train on"
    );

    let id = a_message(&connection, inbox, account.id);
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(a_body(1)),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("set");

    assert_eq!(
        body::train_dictionary(&connection).expect("train"),
        None,
        "one message is not a corpus"
    );
}

#[test]
fn a_body_survives_being_rewritten() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let id = a_message(&connection, inbox, account.id);
    let messages = MessageRepository::new(&connection);

    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("first".to_owned()),
                html: Some("<p>first</p>".to_owned()),
                headers: None,
            },
            BodyState::Partial,
        )
        .expect("set");
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some("second".to_owned()),
                html: None,
                headers: Some("Subject: x\r\n".to_owned()),
            },
            BodyState::Full,
        )
        .expect("set");

    assert_eq!(
        messages.body(id).expect("body"),
        Some(StoredBody {
            text: Some("second".to_owned()),
            html: None,
            headers: Some("Subject: x\r\n".to_owned()),
        }),
        "a rewrite replaces every part, including clearing one"
    );
}

#[test]
fn text_that_is_not_ascii_round_trips() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (account, inbox) = test_support::account_with_inbox(&connection);
    let id = a_message(&connection, inbox, account.id);
    let messages = MessageRepository::new(&connection);

    let text = "Grüße aus München — 日本語 — 🎉 — ¿qué tal?".repeat(20);
    messages
        .set_body(
            id,
            &StoredBody {
                text: Some(text.clone()),
                ..StoredBody::default()
            },
            BodyState::Full,
        )
        .expect("set");

    assert_eq!(
        messages.body(id).expect("body").expect("the row").text,
        Some(text)
    );
}

#[test]
fn a_body_on_a_row_that_does_not_exist_is_an_error() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let messages = MessageRepository::new(&connection);

    assert!(
        messages
            .body(MessageId::new(9_999))
            .expect("body")
            .is_none()
    );
    assert!(
        messages
            .set_body(
                MessageId::new(9_999),
                &StoredBody::default(),
                BodyState::Full
            )
            .is_err()
    );
}
