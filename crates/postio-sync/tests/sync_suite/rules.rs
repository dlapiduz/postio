//! The two evaluation points, and that neither does the other's work (#482).
//!
//! ADR 0008 Q3. A header-only rule runs in the sync pass that inserts the
//! message; a body-requiring rule runs when that message's body lands. The
//! classification is `postio_search::rules`' and is unit-tested there. What
//! needs a sync pass and a backend is the join: that each point actually
//! evaluates the rules it owns, at the moment it owns them, and that no
//! message is evaluated against the same rule twice.
//!
//! # Why a delayed body is the fixture
//!
//! Because the bug this prevents is invisible when bodies are already
//! local. `MockBackend` serves headers on the sync pass and bodies only when
//! a fetch asks — which is exactly what a real server plus ADR 0016's lazy
//! backfill does — so a body-requiring rule evaluated at the arrival point
//! would match against nothing, come out `false`, and file the mail on it.
//! That is the failure ADR 0008 Q3 was written to prevent, and it only shows
//! up when the body is genuinely absent at the first point.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::rule::{Rule, RuleSource};
use postio_model::{Mailbox, Uid, UidValidity};
use postio_search::rules::RuleSet;
use postio_storage::BlobStore;
use postio_storage::repository::MessageRepository;
use postio_storage::test_support::{self, TempDatabase};
use postio_sync::backfill::{BodyRequest, Want, fetch_body_with_rules};
use postio_sync::sync_mailbox_with_rules;

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second)
}

/// One message from Ada, whose *body* mentions an invoice and whose headers
/// do not. Both rules below therefore select this message, and which of them
/// can say so depends entirely on whether the body is local.
fn note() -> Vec<u8> {
    "From: Ada Lovelace <ada@example.com>\r\n\
     Subject: Note one\r\n\
     Message-ID: <note-1@example.com>\r\n\
     Content-Type: text/plain; charset=utf-8\r\n\
     \r\n\
     Attached is the invoice you asked for.\r\n"
        .to_string()
        .into_bytes()
}

async fn server() -> MockBackend {
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(MockMessage::new(note()).with_internal_date(at(1)));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");
    backend
}

struct Local {
    database: TempDatabase,
    connection: postio_storage::PooledConnection,
    blobs: BlobStore,
    inbox: Mailbox,
}

fn local() -> Local {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);
    let blobs = BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    Local {
        database,
        connection,
        blobs,
        inbox,
    }
}

fn rule(name: &str, query: &str) -> Rule {
    Rule::parse(
        &RuleSource {
            name: name.to_owned(),
            query: Some(query.to_owned()),
            actions: vec!["flag".to_owned()],
            ..RuleSource::default()
        },
        |_| None,
    )
    .expect("a rule")
}

/// Both rules select the fixture message; only one of them can be answered
/// before its body is local.
fn rules() -> RuleSet {
    RuleSet::compile(
        &[
            rule("from-ada", "from:ada"),
            rule("invoices", "body:invoice"),
        ],
        at(0).date_naive(),
    )
}

fn fired(report: &postio_sync::Report) -> Vec<&str> {
    report.fired.iter().map(|hit| hit.rule.as_str()).collect()
}

#[tokio::test]
async fn the_header_rule_fires_on_arrival_and_the_body_rule_waits() {
    let backend = server().await;
    let local = local();
    let rules = rules();

    // ── the arrival point ────────────────────────────────────────────────
    let report = sync_mailbox_with_rules(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        &rules,
        |_| {},
    )
    .await
    .expect("headers");

    assert_eq!(report.inserted, 1, "the fixture message has to land");
    assert_eq!(
        fired(&report),
        vec!["from-ada"],
        "the header rule runs in the pass that inserts the message, and the \
         body rule must not: its body is not local, so it would evaluate to \
         `false` and file the mail on that"
    );
    assert!(
        backend.body_fetches().is_empty(),
        "nothing fetched a body to answer a rule -- doing so would throw away \
         ADR 0016's lazy backfill for every message that arrives"
    );

    // ── the body point ───────────────────────────────────────────────────
    let messages = MessageRepository::new(&local.connection);
    let stored = messages
        .by_uid(
            local.inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(1),
        )
        .expect("look up")
        .expect("stored");

    let fetch = fetch_body_with_rules(
        &local.connection,
        &local.blobs,
        &backend,
        &BodyRequest {
            message: stored.id,
            mailbox: local.inbox.id,
            path: local.inbox.path.clone(),
            uid: Uid::new(1),
            remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:1")),
            size: note().len() as u64,
            received_at: at(1),
            want: Want::Text,
        },
        None,
        &rules,
        &CancelToken::new(),
    )
    .await
    .expect("the body arrives");

    assert_eq!(
        fetch
            .fired
            .iter()
            .map(|hit| hit.rule.as_str())
            .collect::<Vec<_>>(),
        vec!["invoices"],
        "the body rule runs when the body lands, and the header rule must \
         not run again -- it already ran on arrival"
    );
}

/// The body half of "exactly once".
///
/// On the arrival side the guard is structural — `is_new`, the same
/// predicate contact recording uses. The body side has no such thing unless
/// it is written: the backfill queue is derived from `body_state <> 'full'`
/// and so asks for each body once, but that is the *queue* being
/// well-behaved, not this function being safe to call. A second fetch of a
/// message whose body is already local must not fire its rules again, or
/// "exactly once" holds only for as long as nothing else ever asks for a
/// body.
#[tokio::test]
async fn a_body_that_was_already_local_does_not_fire_its_rules_again() {
    let backend = server().await;
    let local = local();
    let rules = rules();

    sync_mailbox_with_rules(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        &rules,
        |_| {},
    )
    .await
    .expect("headers");

    let stored = MessageRepository::new(&local.connection)
        .by_uid(
            local.inbox.id,
            postio_model::Generation::new(VALIDITY),
            Uid::new(1),
        )
        .expect("look up")
        .expect("stored");

    let request = BodyRequest {
        message: stored.id,
        mailbox: local.inbox.id,
        path: local.inbox.path.clone(),
        uid: Uid::new(1),
        remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:1")),
        size: note().len() as u64,
        received_at: at(1),
        want: Want::Text,
    };

    let first = fetch_body_with_rules(
        &local.connection,
        &local.blobs,
        &backend,
        &request,
        None,
        &rules,
        &CancelToken::new(),
    )
    .await
    .expect("the body arrives");
    assert_eq!(
        first.fired.len(),
        1,
        "the body rule has to fire the first time, or the second assertion \
         below cannot fail"
    );

    let again = fetch_body_with_rules(
        &local.connection,
        &local.blobs,
        &backend,
        &request,
        None,
        &rules,
        &CancelToken::new(),
    )
    .await
    .expect("the body arrives again");

    assert!(
        again.fired.is_empty(),
        "fetching a body that was already local fired its rules a second \
         time: {:?}. A rule with `move:` would move the message twice.",
        again
            .fired
            .iter()
            .map(|hit| hit.rule.as_str())
            .collect::<Vec<_>>()
    );
    let _ = local.database;
}

#[tokio::test]
async fn a_message_seen_again_is_not_evaluated_again() {
    let backend = server().await;
    let local = local();
    let rules = rules();

    let first = sync_mailbox_with_rules(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        &rules,
        |_| {},
    )
    .await
    .expect("the first pass");
    assert_eq!(fired(&first), vec!["from-ada"]);

    // The same mailbox, enumerated again — a resync re-fetches messages it
    // already has, which is its whole point. A rule that fired on the first
    // pass must not fire again on the second: the message did not arrive
    // twice, it was merely seen twice.
    let second = sync_mailbox_with_rules(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        &rules,
        |_| {},
    )
    .await
    .expect("the second pass");

    assert!(
        fired(&second).is_empty(),
        "re-seeing a message fired its rules a second time: {:?}. A rule with \
         `move:` would move mail the user had since put back.",
        fired(&second)
    );
    let _ = local.database;
}
