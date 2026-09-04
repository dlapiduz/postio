//! One language, two evaluators, and the test that keeps them one language.
//!
//! ADR 0008 Q1 decides that rules need a second evaluator: the FTS5 executor
//! runs over a database, and a rule fires on a message that is not in one
//! yet. It also says what makes that safe — *"index the whole corpus, run
//! every query in a fixture list through both paths, and assert the result
//! sets are identical. That test is the reason this design is safe, and it is
//! the first thing to write."*
//!
//! This is that test. Two evaluators of one language that disagree is worse
//! than not having rules at all, because a dry-run would show one answer and
//! the rule would then do another — and nothing else in this workspace can
//! see it happen. Every other test here exercises one evaluator, and each of
//! them passes while the two drift apart.
//!
//! # What the corpus supplies, and what this file supplies
//!
//! `postio-model`'s `.eml` corpus is real mail: senders with diacritics,
//! encoded-word subjects, `List-Id`s, attachments, multipart bodies, header
//! blocks with `Received` chains. What it does not vary is receipt time (one
//! sentinel for every fixture), flags, or size — so this file spreads those
//! across the corpus, because an operator whose every message answers the
//! same way is an operator this test is not exercising.
//!
//! # Reading a failure
//!
//! A disagreement names the query, the fixture, and which side said yes. That
//! is deliberately more than "assert_eq failed": the two evaluators are
//! thousands of lines apart, and the fixture name is what turns a failure
//! into a reproduction.

use chrono::{DateTime, Duration, TimeZone, Utc};
use postio_model::{AccountScope, BodyState, Flag, Message, MessageId};
use postio_search::facets::Scope;
use postio_search::matcher::{Subject, matches};
use postio_search::{ParsedQuery, parse};
use postio_storage::repository::{MessageRepository, StoredBody};
use postio_storage::test_support;
use rusqlite::Connection;
use std::collections::BTreeSet;

/// The spellings `in:` and `account:` accept for the store this test built.
///
/// **Read off the rows rather than written down here**, because the harness
/// asserting agreement must not be the thing that decides what the two
/// evaluators are agreeing about. Hard-coding a display name the store does
/// not have produced 41 spurious disagreements the first time this ran, all
/// of them the harness's own fault; that is the failure mode a differential
/// test is most prone to, since only one side is being told.
struct Names {
    account: Vec<String>,
}

impl Names {
    fn account(&self) -> Vec<&str> {
        self.account.iter().map(String::as_str).collect()
    }
}

/// The spellings `in:` accepts for one mailbox: its name, its path, its role.
fn mailbox_names(mailbox: &postio_model::Mailbox) -> Vec<String> {
    vec![
        mailbox.name.clone(),
        mailbox.path.clone(),
        mailbox.role.as_str().to_owned(),
    ]
}

fn today() -> chrono::NaiveDate {
    chrono::NaiveDate::from_ymd_opt(2026, 8, 22).expect("a real date")
}

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 8, 22, 12, 0, 0).unwrap()
}

/// One corpus message as both evaluators will see it.
struct Indexed {
    fixture: &'static str,
    message: Message,
    body: Option<String>,
    /// This message's own mailbox spellings. Per message rather than shared,
    /// because the corpus is split across two folders: with one folder every
    /// `in:` query is either total or empty, and an operator whose every
    /// message answers the same way is an operator this file is not testing.
    mailbox: Vec<String>,
}

/// Every query the two evaluators are held to agreement on.
///
/// Chosen to hit each operator on **both** answers — a query that matches
/// everything or nothing agrees trivially, and would leave the operator
/// untested while looking covered. The compositions at the end are there
/// because `OR` and negation are where a boolean tree and a `WHERE` clause
/// are most likely to part company.
const QUERIES: &[&str] = &[
    // Free text, which reaches both indexes.
    "",
    "the",
    "interlock",
    "reunion",
    "réunion",
    "\"tide gate interlock\"",
    "-interlock",
    // Metadata operators, positive and negative.
    "from:ada",
    "from:example.com",
    "from:nobody",
    "to:ada",
    "to:example.com",
    "subject:re",
    "subject:interlock",
    "subject:\"a subject no fixture has\"",
    "filename:pdf",
    "filename:layout",
    "filename:invoice",
    "list:harbour",
    "list:nothing",
    "has:attach",
    "-has:attach",
    "is:unread",
    "is:read",
    "is:flagged",
    // Dates and sizes, spread across the corpus below.
    "after:2026-08-25",
    "after:2026-09-01",
    "before:2026-08-25",
    "before:2020-01-01",
    "larger:1000",
    "smaller:1000",
    // The facts that live outside the message.
    "in:archive",
    "in:\"INBOX/Archive\"",
    "in:inbox",
    "in:\"INBOX/Reference\"",
    "account:test",
    "account:test@example.com",
    "account:personal",
    // ADR 0025's operator, both halves.
    "header:x-mailer",
    "header:list-id",
    "header:content-type",
    "header:content-type=text/plain",
    "header:content-type=multipart",
    "header:x-mail",
    "header:subject=interlock",
    "-header:x-mailer",
    // `body:`, which is the one that needs the body to be local.
    "body:interlock",
    "body:the",
    "body:nothingatallhere",
    // Composition: this is where a tree and a WHERE clause part company.
    "from:ada is:unread",
    "from:ada OR from:grace",
    "from:nobody OR has:attach",
    "(from:ada OR has:attach) is:unread",
    "from:ada OR has:attach is:read",
    "-from:ada has:attach",
    "subject:re -has:attach",
    "(is:read OR is:flagged) -has:attach",
    "header:content-type=text/plain OR filename:pdf",
    "in:archive header:x-mailer",
];

/// Loads the corpus, varies the facts it holds constant, and puts it in a
/// store the executor can read.
fn corpus(connection: &Connection) -> (Vec<Indexed>, Names) {
    postio_index::index::ensure_schema(connection).expect("schema");
    let account = test_support::account(connection);
    let archive = test_support::mailbox(connection, &account, "INBOX/Archive");
    let reference = test_support::mailbox(connection, &account, "INBOX/Reference");
    let folders = [
        (&archive, mailbox_names(&archive)),
        (&reference, mailbox_names(&reference)),
    ];
    let messages = MessageRepository::new(connection);
    // Exactly the disjunction `filter_condition` compiles: an account by
    // display name or address.
    let names = Names {
        account: vec![
            account.display_name.clone(),
            account.address.address.clone(),
        ],
    };

    let mut indexed = Vec::new();
    for (nth, fixture) in postio_model::test_corpus::all().iter().enumerate() {
        let (folder, folder_names) = &folders[nth % folders.len()];
        let mut message = fixture.parse();
        message.account_id = account.id;
        message.mailbox_id = folder.id;
        // The corpus gives every fixture one sentinel receipt time, one
        // size and no flags, so every date, size and `is:` query would
        // answer the same way for all 41 and prove nothing. Spread them.
        message.received_at =
            Utc.with_ymd_and_hms(2026, 8, 10, 9, 0, 0).unwrap() + Duration::days(nth as i64);
        message.size = 500 + (nth as u64 * 137);
        if nth % 3 == 0 {
            message.flags.insert(Flag::Seen);
        }
        if nth % 5 == 0 {
            message.flags.insert(Flag::Flagged);
        }
        messages.create(&mut message).expect("create");

        let body = postio_index::index::indexable_text(&message.body);
        let block = postio_model::headers::block_of(fixture.bytes());
        messages
            .set_body(
                message.id,
                &StoredBody {
                    text: message.body.text.clone(),
                    html: message.body.html.clone(),
                    headers: block.as_ref().map(|block| block.text.clone()),
                    headers_truncated: block.as_ref().is_some_and(|block| block.truncated),
                    encoding_problems: false,
                },
                BodyState::Full,
            )
            .expect("store the body");
        postio_index::index::index_body(connection, message.id.get(), body.as_deref())
            .expect("index the body");
        postio_index::index::index_headers(connection, message.id.get(), &message.headers)
            .expect("index the headers");

        indexed.push(Indexed {
            fixture: fixture.name(),
            message,
            body,
            mailbox: folder_names.clone(),
        });
    }
    assert!(
        indexed.len() > 20,
        "the corpus got smaller than this test's premise"
    );
    (indexed, names)
}

/// What the executor selects.
fn executor(
    connection: &Connection,
    account: postio_model::AccountId,
    query: &ParsedQuery,
) -> BTreeSet<MessageId> {
    postio_index::search(
        connection,
        &postio_index::SearchRequest {
            account: AccountScope::Account(account),
            query,
            scope: Scope::AllMail,
            // Past the corpus, so paging can never be mistaken for
            // disagreement.
            limit: 500,
            order: postio_search::ResultOrder::Newest,
        },
        now(),
    )
    .expect("the search runs")
    .hits
    .into_iter()
    .map(|hit| hit.message_id)
    .collect()
}

/// What `matcher` selects, one message at a time, the way a rule would.
fn matcher(
    corpus: &[Indexed],
    names: &Names,
    query: &ParsedQuery,
    matcher: impl Fn(&ParsedQuery, &Subject<'_>) -> bool,
) -> BTreeSet<MessageId> {
    let account = names.account();
    corpus
        .iter()
        .filter(|indexed| {
            let mailbox: Vec<&str> = indexed.mailbox.iter().map(String::as_str).collect();
            let subject = Subject::new(&indexed.message)
                .with_body(indexed.body.as_deref())
                .in_mailbox(&mailbox)
                .in_account(&account);
            matcher(query, &subject)
        })
        .map(|indexed| indexed.message.id)
        .collect()
}

/// Every place the two evaluators part company, named well enough to
/// reproduce.
///
/// Takes the matcher as a parameter, which is what lets
/// [`the_harness_notices_a_matcher_that_is_wrong`] hand it a broken one and
/// prove this comparison can fail. A differential test that cannot detect a
/// difference is worth nothing, and looks exactly like one that can.
fn disagreements(
    connection: &Connection,
    account: postio_model::AccountId,
    corpus_rows: &[Indexed],
    names: &Names,
    matcher_under_test: impl Fn(&ParsedQuery, &Subject<'_>) -> bool + Copy,
) -> Vec<String> {
    let by_id: std::collections::BTreeMap<MessageId, &str> = corpus_rows
        .iter()
        .map(|indexed| (indexed.message.id, indexed.fixture))
        .collect();

    let mut found = Vec::new();
    for text in QUERIES {
        let query = parse(text, today());
        let from_sql = executor(connection, account, &query);
        let from_memory = matcher(corpus_rows, names, &query, matcher_under_test);

        for id in from_sql.difference(&from_memory) {
            found.push(format!(
                "{text:?}: the executor selected {} and the matcher did not",
                by_id.get(id).copied().unwrap_or("an unknown message")
            ));
        }
        for id in from_memory.difference(&from_sql) {
            found.push(format!(
                "{text:?}: the matcher selected {} and the executor did not",
                by_id.get(id).copied().unwrap_or("an unknown message")
            ));
        }
    }
    found
}

#[test]
fn the_two_evaluators_select_the_same_mail_for_every_query() {
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (rows, names) = corpus(&connection);
    let account = rows[0].message.account_id;

    let found = disagreements(&connection, account, &rows, &names, matches);

    assert!(
        found.is_empty(),
        "the search bar and a rule built from the same query text select \
         different mail. That is the failure ADR 0008 Q1 arranged this whole \
         design to prevent, because a dry-run would show one answer and the \
         rule would do another.\n\n{}",
        found.join("\n")
    );
}

#[test]
fn every_query_in_the_list_is_answered_by_something_and_refused_by_something() {
    // The premise the test above rests on. A query that matches all 41
    // fixtures, or none of them, agrees trivially — so an operator covered
    // only by such a query is an operator this file is not testing while
    // appearing to. Three are exempt and named: the empty query selects
    // everything by definition, and two are here precisely to assert that a
    // value nothing carries finds nothing.
    const ALLOWED_TO_BE_TOTAL: &[&str] = &[
        // Selects everything by definition.
        "",
        // Every fixture is a MIME message, and every account-scoped search is
        // already scoped to this account, so these are total for a reason
        // about the corpus rather than about the operator. They still assert
        // agreement, which is what this file is for.
        "header:content-type",
        "account:test",
        "account:test@example.com",
    ];
    const ALLOWED_TO_BE_EMPTY: &[&str] = &[
        // Each of these is here precisely to assert that a value nothing
        // carries finds nothing — in both evaluators.
        "from:nobody",
        "subject:\"a subject no fixture has\"",
        "list:nothing",
        "before:2020-01-01",
        "in:inbox",
        "account:personal",
        "header:x-mail",
        "body:nothingatallhere",
        "filename:invoice",
    ];

    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (rows, _names) = corpus(&connection);
    let account = rows[0].message.account_id;

    let mut useless = Vec::new();
    for text in QUERIES {
        let query = parse(text, today());
        let hits = executor(&connection, account, &query).len();
        if hits == rows.len() && !ALLOWED_TO_BE_TOTAL.contains(text) {
            useless.push(format!("{text:?} matched every fixture"));
        }
        if hits == 0 && !ALLOWED_TO_BE_EMPTY.contains(text) {
            useless.push(format!("{text:?} matched no fixture"));
        }
    }
    assert!(
        useless.is_empty(),
        "these queries cannot tell the two evaluators apart, so they are \
         covering their operators in appearance only. Either give the corpus \
         something to disagree about or name the query in the exemptions \
         above and say why.\n{}",
        useless.join("\n")
    );
}

#[test]
fn the_harness_notices_a_matcher_that_is_wrong() {
    // The control, and the acceptance criterion in its own right: a
    // differential test that cannot detect a difference is worth nothing and
    // looks exactly like one that can.
    //
    // The mutation is the most plausible one available rather than an
    // arbitrary flip — `from:ad` finding `ada@example.com` — because a
    // matcher written without the executor in front of you is *substring*
    // matching, and every unit test of the matcher alone would still pass.
    let database = test_support::memory();
    let connection = database.connection().expect("checkout");
    let (rows, names) = corpus(&connection);
    let account = rows[0].message.account_id;

    let substring_matcher = |query: &ParsedQuery, subject: &Subject<'_>| {
        for clause in query.filters() {
            if let postio_search::query::Filter::From(value) = &clause.filter {
                let senders = subject
                    .message()
                    .from
                    .iter()
                    .map(|address| {
                        format!(
                            "{} {}",
                            address.name.as_deref().unwrap_or_default(),
                            address.address
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(" ")
                    .to_lowercase();
                return clause.negated ^ senders.contains(&value.to_lowercase());
            }
        }
        matches(query, subject)
    };

    let found = disagreements(&connection, account, &rows, &names, substring_matcher);

    assert!(
        !found.is_empty(),
        "a matcher that treats `from:` as a substring rather than a token \
         search agreed with the executor on every query in the list. The \
         comparison above is therefore not measuring what it claims, and the \
         query list needs a case that tells the two apart."
    );
}
