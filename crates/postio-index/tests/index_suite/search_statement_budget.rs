//! POSTIO-MEASUREMENT: each case indexes a corpus large enough for a non-flat statement count to
//! show (83 s and 106 s), so they run on the nightly timer with `total_hits_cap`.

use chrono::{TimeZone, Utc};
use postio_index::{SearchRequest, search};
use postio_model::AccountScope;
use postio_model::{EmailAddress, Message};
use postio_search::facets::Scope;
use postio_search::parse;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support;
use postio_storage::test_support::counting::{Counts, counted_async, install};

const MATCHES: usize = 2_500;

async fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 21).expect("a real date")
}

#[tokio::test]
async fn a_search_costs_the_same_queries_however_much_it_matches() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, mailbox) = test_support::account_with_inbox(&connection).await;

    let messages = MessageRepository::new(&connection);
    for nth in 0..MATCHES {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            + chrono::Duration::seconds(nth as i64);
        let mut message = Message::new(account.id, mailbox, received);
        message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly report {nth}"));
        messages.create(&mut message).await.expect("create message");
        let headers: postio_model::Headers = [("X-Mailer", format!("Mutt 1.5.24 build {nth}"))]
            .into_iter()
            .collect();
        postio_index::index::index_headers(&connection, message.id.get(), &headers)
            .await
            .expect("index headers");
    }

    let broad_query = parse("quarterly", today().await);
    let narrow_query = parse(&format!("{}", MATCHES - 1), today().await);
    let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();

    install(&connection);
    let run = async |query, limit| {
        let request = SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(account.id),
            query,
            scope: Scope::AllMail,
            limit,
            order: postio_search::ResultOrder::Relevance,
        };
        let mut hits = 0;
        let counts: Counts = counted_async(async || {
            hits = search(&connection, &request, now)
                .await
                .expect("a page of results")
                .hits
                .len();
        })
        .await;
        (counts, hits)
    };

    let (broad, broad_hits) = run(&broad_query, 25).await;
    let (narrow, narrow_hits) = run(&narrow_query, 25).await;

    assert_eq!(broad_hits, 25, "a broad query should fill a page of 25");
    assert!(
        (1..25).contains(&narrow_hits),
        "the narrow query matched {narrow_hits} messages; it is meant to match \
         a handful, so that it and the broad query are genuinely different \
         amounts of work"
    );
    assert_eq!(
        broad.statements, narrow.statements,
        "a query matching {MATCHES} messages issued {} statements where one \
         matching {narrow_hits} issued {}. Search that costs more queries the \
         more it matches is what §18's 100ms budget cannot survive, and it \
         returns the same results either way, so nothing else here notices.",
        broad.statements, narrow.statements
    );

    let with_one_more = counted_async(async || {
        let request = SearchRequest {
            mailboxes: None,
            account: AccountScope::Account(account.id),
            query: &broad_query,
            scope: Scope::AllMail,
            limit: 25,
            order: postio_search::ResultOrder::Relevance,
        };
        let _ = search(&connection, &request, now)
            .await
            .expect("a page of results");
        let _: i64 =
            postio_storage::sql::one(&connection, "SELECT count(*) FROM messages", (), |row| {
                postio_storage::sql::RowExt::col(row, 0)
            })
            .await
            .expect("one more query, standing in for a per-hit lookup");
    });
    assert_eq!(
        with_one_more.await.statements,
        broad.statements + 1,
        "adding a query to the counted block did not change the count, so the \
         equalities either side of this are not measuring what they claim"
    );

    let header_broad_query = parse("header:x-mailer=mutt", today().await);
    let header_narrow_query = parse(
        &format!("header:x-mailer=\"build {}\"", MATCHES - 1),
        today().await,
    );
    let (header_broad, header_broad_hits) = run(&header_broad_query, 25).await;
    let (header_narrow, header_narrow_hits) = run(&header_narrow_query, 25).await;

    assert_eq!(
        header_broad_hits, 25,
        "`header:x-mailer=mutt` is on every message, so it should fill a page"
    );
    assert_eq!(
        header_narrow_hits, 1,
        "and the numbered value should pick out exactly one"
    );
    assert_eq!(
        header_broad.statements, header_narrow.statements,
        "a `header:` matching {MATCHES} messages issued {} statements where \
         one matching {header_narrow_hits} issued {}. The EXISTS is correlated \
         to the outer message, so a plan that lost `idx_message_headers_name` \
         would read the whole table per row and return the same results.",
        header_broad.statements, header_narrow.statements
    );

    let (header_wide, header_wide_hits) = run(&header_broad_query, 100).await;
    assert_eq!(header_wide_hits, 100, "a page of 100 should come back full");
    assert_eq!(
        header_wide.statements, header_broad.statements,
        "showing {header_wide_hits} `header:` hits issued {} statements where \
         {header_broad_hits} issued {}: a query per hit.",
        header_wide.statements, header_broad.statements
    );

    let (wide, wide_hits) = run(&broad_query, 100).await;
    assert_eq!(wide_hits, 100, "a page of 100 should come back full");
    assert_eq!(
        wide.statements, broad.statements,
        "showing {wide_hits} hits issued {} statements where {broad_hits} \
         issued {}. That is a query per hit, which is invisible in the results \
         and fatal to the budget.",
        wide.statements, broad.statements
    );
}

#[tokio::test]
async fn the_facets_walk_the_match_once_per_scope() {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let archive = test_support::mailbox(&connection, &account, "Archive")
        .await
        .id;
    let messages = MessageRepository::new(&connection);
    for nth in 0..30 {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            + chrono::Duration::seconds(nth as i64);
        let folder = if nth % 3 == 0 { archive } else { inbox };
        let mut message = Message::new(account.id, folder, received);
        message.from = vec![EmailAddress::new(Some("ada"), "ada@example.com")];
        message.subject = Some(format!("quarterly report {nth}"));
        if nth % 5 == 0 {
            message.flags = [postio_model::Flag::Flagged].into_iter().collect();
        }
        messages.create(&mut message).await.expect("create message");
    }
    let query = parse("quarterly", today().await);
    let request = SearchRequest {
        mailboxes: None,
        account: AccountScope::Account(account.id),
        query: &query,
        scope: Scope::AllMail,
        limit: 25,
        order: postio_search::ResultOrder::Relevance,
    };
    install(&connection);
    let mut facets = None;
    let counts: Counts = counted_async(async || {
        facets = Some(
            postio_index::executor::facets(&connection, &request)
                .await
                .expect("facets"),
        );
    })
    .await;
    let facets = facets.expect("facets ran");

    let hits = |scope: Scope| {
        facets
            .scopes
            .iter()
            .find(|count| count.scope == scope)
            .map(|count| count.hits)
    };
    assert_eq!(hits(Scope::AllMail), Some(30), "every message matched");
    assert_eq!(
        hits(Scope::Inbox),
        Some(20),
        "two in three are in the inbox"
    );
    let refinement = |token: &str| {
        facets
            .refinements
            .iter()
            .find(|refinement| refinement.token == token)
            .map(|refinement| refinement.hits)
    };
    assert_eq!(refinement("is:flagged"), Some(6));
    assert_eq!(refinement("in:INBOX"), Some(20));
    assert_eq!(refinement("in:Archive"), Some(10));
    assert!(
        counts.statements <= 3,
        "the facets took {} statements; the match is walked once per scope and \
         the refinements share the current scope's walk",
        counts.statements
    );
}

async fn conversation_corpus(size: usize) -> (postio_storage::Store, postio_storage::Checkout) {
    let database = test_support::memory().await;
    let connection = database.connect().await.expect("checkout");
    postio_index::index::ensure_schema(&connection)
        .await
        .expect("schema");
    let (account, inbox) = test_support::account_with_inbox(&connection).await;
    let mut atlas = postio_model::Label::new(account.id, "Atlas");
    let atlas = postio_storage::repository::LabelRepository::new(&connection)
        .create(&mut atlas)
        .await
        .expect("a label");

    connection.execute("BEGIN", ()).await.expect("begin");
    postio_index::index::defer_documents(&connection)
        .await
        .expect("defer the index");
    let messages = MessageRepository::new(&connection);
    let threads = postio_storage::repository::ThreadRepository::new(&connection);
    let mut ids = Vec::with_capacity(size);
    let mut thread = None;
    for nth in 0..size {
        let received = Utc.with_ymd_and_hms(2026, 8, 20, 9, 0, 0).unwrap()
            - chrono::Duration::hours(nth as i64);
        let mut message = Message::new(account.id, inbox, received);
        let sender = format!("sender{}", nth % 4);
        message.from = vec![EmailAddress::new(
            Some(sender.as_str()),
            format!("{sender}@example.com"),
        )];
        message.to = vec![EmailAddress::new(Some("Me"), "me@example.com")];
        message.subject = Some(format!("Weekly report {nth}"));
        if nth % 10 == 0 {
            let mut sheet = postio_model::Attachment::new(
                postio_model::MessageId::UNASSIGNED,
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                4_000,
            );
            sheet.filename = Some(format!("sheet-{nth}.xlsx"));
            message.attachments.push(sheet);
        }
        messages.create(&mut message).await.expect("create message");
        if nth % 3 == 0 {
            let mut created = postio_model::Thread::new(account.id);
            thread = Some(threads.create(&mut created).await.expect("a thread"));
        }
        threads
            .add_message(thread.expect("a thread"), message.id)
            .await
            .expect("join it");
        if nth % 4 == 0 {
            postio_storage::repository::LabelRepository::new(&connection)
                .attach(message.id, atlas)
                .await
                .expect("label it");
        }
        let body = if nth % 5 == 0 {
            format!("an update on the budget, number {nth}")
        } else {
            format!("an update, number {nth}")
        };
        postio_index::index::index_body(&connection, message.id.get(), Some(&body))
            .await
            .expect("index body");
        ids.push(message.id);
    }
    postio_index::index::write_documents(&connection, &ids)
        .await
        .expect("index the documents");
    let sheets: Vec<i64> = postio_storage::sql::all_unbounded(
        &connection,
        "SELECT id FROM attachments ORDER BY id",
        (),
        |row| {
            use postio_storage::sql::RowExt as _;
            row.col(0)
        },
    )
    .await
    .expect("the sheets");
    for sheet in sheets {
        let unit = |row: u32, text: String| postio_extract::Unit {
            location: postio_extract::Location::Sheet {
                name: "Summary".to_owned(),
                row,
            },
            text,
        };
        postio_index::index::index_attachment_text(
            &connection,
            postio_model::AttachmentId::new(sheet),
            &postio_extract::Extracted {
                units: (1..=20)
                    .map(|row| unit(row, format!("kestrel survey line {row}")))
                    .collect(),
                outcome: postio_extract::Outcome::Complete,
            },
        )
        .await
        .expect("index the sheet's text");
    }
    connection.execute("COMMIT", ()).await.expect("commit");
    (database, connection)
}

#[tokio::test]
async fn a_conversation_search_costs_the_same_statements_whatever_the_corpus() {
    use postio_index::executor::{ConversationRequest, search_conversations};
    use postio_search::results::ConversationOrder;

    const SHAPES: [&str; 8] = [
        "budget",
        "from:sender1",
        "from:sender1 budget",
        "update",
        "label:atlas budget",
        "budget -sender2",
        "kestrel",
        "kestrel from:sender0",
    ];
    let now = Utc.with_ymd_and_hms(2026, 8, 21, 12, 0, 0).unwrap();
    let mut costs: Vec<Vec<usize>> = Vec::new();
    for size in [100, 2_000] {
        let (_database, connection) = conversation_corpus(size).await;
        install(&connection);
        let mut row = Vec::new();
        for shape in SHAPES {
            for (order, limit) in [
                (ConversationOrder::BestMatch, 4),
                (ConversationOrder::BestMatch, 50),
                (ConversationOrder::Newest, 50),
            ] {
                let query = parse(shape, today().await);
                let request = ConversationRequest {
                    account: AccountScope::Unified,
                    query: &query,
                    order,
                    offset: 0,
                    limit,
                    today: today().await,
                };
                let mut total = 0;
                let counts: Counts = counted_async(async || {
                    total = search_conversations(&connection, &request, now)
                        .await
                        .expect("a conversation search")
                        .total;
                })
                .await;
                assert!(total > 0, "{shape:?} matched nothing in {size}");
                assert!(
                    counts.statements <= 5,
                    "{shape:?} ({order:?}, {limit}) took {} statements over {size} \
                     messages; research R2's budget is five: the walk, the attachment \
                     text, the page's checks, the hydrate, the folders",
                    counts.statements
                );
                row.push(counts.statements);
            }
        }
        costs.push(row);
    }
    assert_eq!(
        costs[0], costs[1],
        "statements per shape over 100 messages, then over 2,000 ({SHAPES:?}, three \
         orders and pages each): a conversation search that reads more often the more \
         mail there is cannot keep its 50 ms"
    );
}
