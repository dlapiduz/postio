use postio_index::index::{HEADER_ROWS_PER_MESSAGE, ensure_schema, index_body, index_headers};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::bind;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

const BYTES_PER_MESSAGE: i64 = 5 * 1024;

const MESSAGES: usize = 400;

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

fn a_block(n: usize) -> postio_model::Headers {
    let signature = format!(
        "v=1; a=rsa-sha256; c=relaxed/relaxed; d=example.com; s=selector{n}; \
         h=from:to:subject:date:message-id:mime-version:content-type; \
         bh={}; b={}",
        "b".repeat(44),
        "c".repeat(700)
    );
    let mut headers = postio_model::Headers::new();
    for hop in 0..3 {
        headers.push(
            "Received",
            format!(
                "from relay{hop}.example.net (relay{hop}.example.net [192.0.2.{hop}]) by \
                 mx.example.com with ESMTPS id abcdef{n}{hop}; Mon, 3 Aug 2026 09:0{hop}:00 +0000"
            ),
        );
    }
    headers.push("Return-Path", "<ada@example.com>");
    headers.push(
        "Authentication-Results",
        "mx.example.com; spf=pass; dkim=pass; dmarc=pass",
    );
    headers.push("DKIM-Signature", signature.clone());
    headers.push("ARC-Seal", signature.clone());
    headers.push("ARC-Message-Signature", signature);
    headers.push(
        "ARC-Authentication-Results",
        "i=1; mx.example.com; spf=pass",
    );
    headers.push("From", "Ada Lovelace <ada@example.com>");
    headers.push("To", "Engine Programme <programme@lists.example.org>");
    headers.push("Subject", format!("Re: engine notes {n}"));
    headers.push("Date", "Mon, 3 Aug 2026 09:00:00 +0000");
    headers.push("Message-ID", format!("<engine-{n}@example.com>"));
    headers.push(
        "In-Reply-To",
        format!("<engine-{}@example.com>", n.saturating_sub(1)),
    );
    headers.push("MIME-Version", "1.0");
    headers.push("Content-Type", "text/plain; charset=utf-8");
    headers.push("Content-Transfer-Encoding", "quoted-printable");
    headers.push("List-Id", "Engine Programme <programme.lists.example.org>");
    headers.push(
        "List-Unsubscribe",
        "<mailto:programme-unsubscribe@lists.example.org>",
    );
    headers.push("Precedence", "list");
    headers.push("X-Mailer", "Mutt 1.5.24 (2015-08-30)");
    headers
}

async fn store_bytes(store: &postio_storage::test_support::TempStore) -> i64 {
    store.truncate_log().await.expect("fold the log back in");
    std::fs::metadata(store.directory().join("postio.db"))
        .expect("the store is on disk")
        .len() as i64
}

#[tokio::test]
async fn no_message_may_contribute_more_than_the_two_caps_allow() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
    messages.create(&mut message).await.expect("create");
    let mut headers = postio_model::Headers::new();
    for hop in 0..100 {
        headers.push(
            "Received",
            format!("from relay{hop}.example.net {}", "x".repeat(4096)),
        );
    }
    index_headers(&connection, message.id.get(), &headers)
        .await
        .expect("index");

    let (rows, longest): (i64, i64) = postio_storage::sql::one(
        &connection,
        "SELECT count(*), coalesce(max(length(value)), 0) FROM message_headers
              WHERE content_id = (SELECT content_id FROM messages WHERE id = ?1)",
        bind![message.id.get()],
        |row| {
            Ok((
                postio_storage::sql::RowExt::col(row, 0)?,
                postio_storage::sql::RowExt::col(row, 1)?,
            ))
        },
    )
    .await
    .expect("measure");

    assert_eq!(
        rows, HEADER_ROWS_PER_MESSAGE as i64,
        "one message put {rows} rows in the table; the cap is what stops a \
         twenty-hop mailing-list message from deciding the size of the store"
    );
    assert!(
        longest <= postio_model::headers::VALUE_LIMIT as i64,
        "a value of {longest} bytes was stored; the cap is {}",
        postio_model::headers::VALUE_LIMIT
    );
}

#[tokio::test]
async fn the_header_index_stays_inside_its_per_message_ceiling() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    let empty = store_bytes(&database).await;

    let mut ids = Vec::with_capacity(MESSAGES);
    connection.execute_batch("BEGIN").await.expect("begin");
    for n in 0..MESSAGES {
        let mut message = Message::new(account.id, mailbox, chrono::Utc::now());
        message.subject = Some(format!("Re: engine notes {n}"));
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.sync.body_state = BodyState::Full;
        messages.create(&mut message).await.expect("create");
        index_body(&connection, message.id.get(), Some(&a_body(n)))
            .await
            .expect("index a body");
        ids.push(message.id.get());
    }
    connection.execute_batch("COMMIT").await.expect("commit");
    let with_bodies = store_bytes(&database).await;

    connection.execute_batch("BEGIN").await.expect("begin");
    for (n, id) in ids.iter().enumerate() {
        index_headers(&connection, *id, &a_block(n))
            .await
            .expect("index headers");
    }
    connection.execute_batch("COMMIT").await.expect("commit");
    let with_headers = store_bytes(&database).await;

    let header_rows: i64 = postio_storage::sql::one(
        &connection,
        "SELECT count(*) FROM message_headers",
        (),
        |row| postio_storage::sql::RowExt::col(row, 0),
    )
    .await
    .expect("count");
    assert!(
        header_rows >= MESSAGES as i64 * 20,
        "the fixture has to be a real block per message, or the ratio below \
         is measuring nothing; got {header_rows} rows"
    );

    let headers = with_headers - with_bodies;
    let bodies = with_bodies - empty;
    assert!(headers > 0, "the header index measured as empty");

    let per_message = headers / MESSAGES as i64;
    let mb = |bytes: i64| bytes as f64 / (1024.0 * 1024.0);
    println!(
        "\n{MESSAGES} messages\n  the headers add          {:>8.2} MB\n  \
         the mail and bodies add  {:>8.2} MB\n  share                    {:>8.1} %\n  \
         per message              {per_message:>8} B\n",
        mb(headers),
        mb(bodies),
        100.0 * headers as f64 / bodies.max(1) as f64,
    );

    assert!(
        per_message <= BYTES_PER_MESSAGE,
        "the header index costs {per_message} B a message, over ADR 0027 Q2's \
         {BYTES_PER_MESSAGE} B ceiling. The lever is the two caps -- \
         `postio_model::headers::VALUE_LIMIT` and \
         `postio_index::index::HEADER_ROWS_PER_MESSAGE` -- not a list of \
         header names, and not shipping it anyway. ADR 0027 Q1 prices them: \
         512 -> 2,793 B/msg, 256 -> 2,025, 128 -> 1,630, 64 -> 1,246, so a cap \
         takes about half once and never an order of magnitude. Bump \
         HEADERS_SCHEMA_VERSION with whichever cap moves, so existing stores \
         are refilled under the new policy."
    );
}
