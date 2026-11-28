use postio_index::index::{ensure_schema, index_body};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

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

/// The log is folded into the file first, or this reads a database whose newest pages are still in
/// `postio.db-wal`.
async fn store_bytes(store: &postio_storage::test_support::TempStore) -> i64 {
    store.truncate_log().await.expect("fold the log back in");
    std::fs::metadata(store.directory().join("postio.db"))
        .expect("the store is on disk")
        .len() as i64
}

#[tokio::main]
async fn main() {
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
