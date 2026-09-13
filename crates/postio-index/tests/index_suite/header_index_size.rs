//! What indexing every header costs, measured against what indexing every
//! body costs (ADR 0025 Q3).
//!
//! Beside `body_index_size.rs`, and asserting where that one only prints.
//! The difference is what each is for: that one records a saving already
//! taken, and this one holds a *policy* to account. ADR 0025 indexes every
//! header with no allowlist, on the argument that the two structural caps —
//! 512 bytes a value, 64 rows a message — keep it at metadata scale rather
//! than corpus scale. That argument is checkable, and if it is wrong the
//! lever is the caps: not a list of header names, and not shipping it anyway.
//!
//! **Relative, not absolute.** The budget is a share of what the body index
//! costs on the same corpus, because that ratio is what stays true on another
//! machine and another mailbox — a byte count would be a number about this
//! test's fixture.
//!
//! **Measured as a file delta, because this engine has no `dbstat`.** The
//! old measurement asked SQLite's `dbstat` virtual table for page usage per
//! b-tree by name. There is no such table here, and no per-object accounting
//! at all -- so the store is weighed on disk before the headers are indexed
//! and again after, and the difference is what the header policy costs.
//!
//! That is a coarser instrument in one way and a truer one in another. It
//! cannot attribute bytes to the table versus its index, which `dbstat`
//! could; but it counts everything the policy actually adds to the file,
//! including the index, the free pages it leaves and any overhead a
//! by-name pattern would have missed. The old one missed exactly that: its
//! `LIKE 'message\_headers\_%'` was written for FTS5's shadow tables and did
//! not match `idx_message_headers_name`, understating the cost by 17%.
//!
//! # The gate is bytes per message, and ADR 0027 is why
//!
//! ADR 0025 Q3 asked for `message_headers` to stay under 25% of
//! `message_bodies_fts`, and that target is unreachable by the levers it
//! named. The cause is structural rather than tuning: `message_bodies_fts` is
//! `content = ''`, an inverted index holding *no text at all*, while
//! `message_headers` holds every value verbatim because a substring match
//! needs the string. Measured on four corpora the share ran 64% to 269%, and
//! moved with **body** length rather than with anything about the header
//! policy -- the committed fixture measures 636% for that reason alone.
//!
//! ADR 0027 amends it. The ceiling is now on the thing the caps actually
//! control:
//!
//! > `message_headers` plus `idx_message_headers_name`, in `dbstat` page
//! > bytes, divided by the number of messages, must stay under 5 KiB.
//!
//! Bytes per message is invariant to the one quantity that differs between
//! mailboxes, and it multiplies straight into disk: 3.72 KiB a message is
//! ~310 MB on ADR 0017's 81,744-message reference account, which is a line
//! item rather than a claim. The ratio survives below as a printed
//! observation and asserts nothing.
//!
//! The fixture is committed and deliberately heavy -- three signature sets, a
//! three-hop `Received` chain, a full mailing-list header set -- so it reads
//! as a ceiling: a lighter mailbox passes by definition.
//!
use postio_index::index::{HEADER_ROWS_PER_MESSAGE, ensure_schema, index_body, index_headers};
use postio_model::{BodyState, EmailAddress, Message};
use postio_storage::bind;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;

/// ADR 0025 Q3's budget: `message_headers` may cost at most this share of
/// what `message_bodies_fts` costs on the same corpus.
/// ADR 0027 Q2: `message_headers` + `idx_message_headers_name`, per message.
///
/// The committed fixture measures **4,218 B** a message, against 3,809 B under
/// SQLCipher -- the same policy on the same fixture, weighed as a file delta
/// rather than by `dbstat`, on an engine with its own page layout. So the
/// ceiling that was a third above the measurement is now about a fifth above
/// it. That is still headroom for a b-tree fanout change or an engine
/// upgrade, and still far too little to absorb a policy change, which is what
/// the number is for.
const BYTES_PER_MESSAGE: i64 = 5 * 1024;

/// Enough messages that the b-trees are more than their root pages and the
/// ratio is about the data rather than about SQLite's overhead. Deliberately
/// far short of `body_index_size.rs`'s 5,000: this one asserts, so it runs in
/// the default suite and has to be worth its seconds.
const MESSAGES: usize = 400;

/// Mail-shaped text: a few hundred words with the repetition real mail has —
/// a quoted parent, a signature, the same handful of names. The same body as
/// `body_index_size.rs`, so the two measurements are about one corpus.
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

/// A header block the shape real mail arrives with: a hop chain, the MIME
/// furniture, a mailing-list set, and the authentication blobs that are most
/// of the bytes and none of the interest — which is exactly what the value
/// cap is for.
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

/// What the store weighs on disk, right now.
///
/// The log is folded back into the file first, or this reads a database whose
/// newest pages are still in `postio.db-wal` -- which is most of them, in the
/// middle of a fixture.
async fn store_bytes(store: &postio_storage::test_support::TempStore) -> i64 {
    store.truncate_log().await.expect("fold the log back in");
    std::fs::metadata(store.directory().join("postio.db"))
        .expect("the store is on disk")
        .len() as i64
}

/// The part that is not in dispute: what one message may contribute.
///
/// ADR 0025 Q3's caps are what keep the cost *bounded* — no message, however
/// pathological, may put more than [`HEADER_ROWS_PER_MESSAGE`] rows or more
/// than `VALUE_LIMIT` bytes a value into the table. That guarantee holds
/// whatever is decided about the ratio below, and it is the one that stops a
/// twenty-hop mailing-list message from deciding the size of the store.
#[tokio::test]
async fn no_message_may_contribute_more_than_the_two_caps_allow() {
    let database = test_support::temp().await;
    let connection = database.connect().await.expect("checkout");
    ensure_schema(&connection).await.expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;
    let messages = MessageRepository::new(&connection);

    // The pathological message: a hundred fields, each far over the cap.
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
        &*connection,
        "SELECT count(*), coalesce(max(length(value)), 0) FROM message_headers
              WHERE message_id = ?1",
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

    // Three weighings of one store, so the delta between them is the cost of
    // exactly the step in between. The headers go in a pass of their own for
    // that reason -- interleaved with the bodies, as the sync path does it,
    // there would be nothing to subtract.
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
    // The ratio is printed and asserts nothing (ADR 0027 Q1): it moves with
    // body length, so on this fixture it says more about `a_body` than about
    // the header policy. Kept because it is the number ADR 0025 Q3 reasoned
    // from, and seeing it move while the per-message figure does not is the
    // clearest statement of why the denominator was wrong.
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
