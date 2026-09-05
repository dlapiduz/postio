//! A rule that selects a message carries its actions out (#481, ADR 0028).
//!
//! `rules.rs` beside this proves *which* rules fire and *when*. This proves
//! the half that comes after: the actions run, they run through the same
//! storage verbs a keystroke runs, and they run inside the transaction that
//! inserted the message — ADR 0008 Q3's "before any event is emitted", so
//! the user never sees the mail land in the Inbox and jump.
//!
//! # What these assert on
//!
//! The stored row and the queued operation, never the report. A pass that
//! announced `flag` and wrote nothing would satisfy an assertion about
//! `report.fired` completely, and that is exactly the shape this issue
//! inherited: evaluation landed in #482 reporting hits outward, with the
//! acting deferred to here.

use chrono::{DateTime, TimeDelta, TimeZone, Utc};
use postio_account::backend::{MailBackend, MockBackend, MockMailbox, MockMessage};
use postio_account::cancel::CancelToken;
use postio_model::rule::{Rule, RuleSource};
use postio_model::{Account, Flag, Label, Mailbox, Operation, RfcMessageId, Uid, UidValidity};
use postio_search::rules::RuleSet;
use postio_storage::repository::{LabelRepository, MessageRepository, OperationQueueRepository};
use postio_storage::test_support::{self, TempDatabase};
use postio_sync::sync_mailbox_with_rules;

const INBOX: &str = "INBOX";
const VALIDITY: u32 = 1_707_000_000;

fn at(second: i64) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 3, 1, 9, 0, 0).unwrap() + TimeDelta::seconds(second)
}

/// One message from Ada, header-answerable so every rule here runs at the
/// arrival point and the fixture needs no body fetch.
fn note() -> Vec<u8> {
    "From: Ada Lovelace <ada@example.com>\r\n\
     Subject: Note one\r\n\
     Message-ID: <note-1@example.com>\r\n\
     Content-Type: text/plain; charset=utf-8\r\n\
     \r\n\
     A short note.\r\n"
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
    #[allow(dead_code)]
    database: TempDatabase,
    connection: postio_storage::PooledConnection,
    account: Account,
    inbox: Mailbox,
}

fn local() -> Local {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);
    Local {
        database,
        connection,
        account,
        inbox,
    }
}

/// A rule selecting the fixture message, carrying `actions` in order.
fn rule(name: &str, actions: &[&str]) -> Rule {
    rule_matching(name, "from:ada", actions)
}

fn rule_matching(name: &str, query: &str, actions: &[&str]) -> Rule {
    Rule::parse(
        &RuleSource {
            name: name.to_owned(),
            query: Some(query.to_owned()),
            actions: actions.iter().map(|a| (*a).to_owned()).collect(),
            ..RuleSource::default()
        },
        |_| None,
    )
    .expect("a rule")
}

fn stopping(name: &str, actions: &[&str]) -> Rule {
    Rule {
        stop: true,
        ..rule(name, actions)
    }
}

fn compile(rules: &[Rule]) -> RuleSet {
    RuleSet::compile(rules, at(0).date_naive())
}

/// The fixture message as it is stored after `rules` have run over it.
async fn arrive(local: &Local, rules: &RuleSet) -> postio_model::Message {
    let backend = server().await;
    let report = sync_mailbox_with_rules(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        rules,
        |_| {},
    )
    .await
    .expect("headers");
    assert_eq!(report.inserted, 1, "the fixture message has to land");
    stored(local)
}

/// The fixture message, found by its RFC id rather than by mailbox and UID.
///
/// Which matters here and not in `rules.rs`: half of these actions *move* the
/// message, and a move nulls the UID it had in the mailbox it left. Looking
/// it up the obvious way would fail on exactly the tests that are working.
fn stored(local: &Local) -> postio_model::Message {
    stored_by_rfc_id(local, "<note-1@example.com>")
}

fn stored_by_rfc_id(local: &Local, rfc_id: &str) -> postio_model::Message {
    let messages = MessageRepository::new(&local.connection);
    let id = *messages
        .ids_by_rfc_message_id(local.account.id, &RfcMessageId::new(rfc_id))
        .expect("look up")
        .first()
        .unwrap_or_else(|| panic!("{rfc_id} is stored"));
    messages
        .get(id)
        .expect("read")
        .unwrap_or_else(|| panic!("{rfc_id} is readable"))
}

fn queued(local: &Local) -> Vec<Operation> {
    OperationQueueRepository::new(&local.connection)
        .pending(local.account.id, at(600))
        .expect("read the queue")
        .into_iter()
        .map(|row| row.operation)
        .collect()
}

#[tokio::test]
async fn the_actions_of_a_matching_rule_reach_the_stored_row() {
    let local = local();
    let message = arrive(&local, &compile(&[rule("triage", &["flag", "mark-read"])])).await;

    assert!(
        message.flags.contains(&Flag::Flagged),
        "`flag` has to reach the row: a rule that reports having flagged a \
         message and writes nothing is the failure this issue exists to \
         close, and it passes every assertion about the report"
    );
    assert!(
        message.flags.contains(&Flag::Seen),
        "`mark-read` has to reach the row too -- the actions run in order and \
         all of them run"
    );
}

#[tokio::test]
async fn an_action_enqueues_the_operation_the_server_needs() {
    let local = local();
    arrive(&local, &compile(&[rule("triage", &["flag"])])).await;

    let flag_writes = queued(&local)
        .into_iter()
        .filter(|operation| matches!(operation, Operation::SetFlags { .. }))
        .count();
    assert_eq!(
        flag_writes, 1,
        "local-first means the write *and* the queue row (ARCHITECTURE.md \
         §1). A rule that files mail only locally is a rule whose effect \
         disappears on the next resync from the server"
    );
}

#[tokio::test]
async fn a_rule_that_does_not_match_acts_on_nothing() {
    let local = local();
    let message = arrive(
        &local,
        &compile(&[rule_matching("others", "from:babbage", &["flag"])]),
    )
    .await;

    assert!(
        !message.flags.contains(&Flag::Flagged),
        "the actions belong to the rules that matched, and only to those"
    );
    assert!(
        queued(&local).is_empty(),
        "and a rule that did not fire enqueues nothing"
    );
}

#[tokio::test]
async fn move_files_the_message_and_tells_the_server_to_as_well() {
    let local = local();
    let receipts = test_support::mailbox(&local.connection, &local.account, "Receipts");
    let message = arrive(&local, &compile(&[rule("file", &["move:Receipts"])])).await;

    assert_eq!(
        message.mailbox_id, receipts.id,
        "`move:` files the message where the rule said"
    );
    assert!(
        queued(&local)
            .iter()
            .any(|operation| matches!(operation, Operation::Move { to, .. } if *to == receipts.id)),
        "and the server is told, or the next resync puts it back"
    );
}

#[tokio::test]
async fn a_move_naming_a_mailbox_that_does_not_exist_leaves_the_mail_alone() {
    let local = local();
    let inbox = local.inbox.id;
    let message = arrive(&local, &compile(&[rule("file", &["move:Nowhere", "flag"])])).await;

    assert_eq!(
        message.mailbox_id, inbox,
        "an unresolvable destination must not move the message somewhere else"
    );
    assert!(
        message.flags.contains(&Flag::Flagged),
        "and it must not stop the actions after it: ADR 0008 Q6 is that an \
         error never drops mail, and failing the pass here would roll back \
         the insert that brought the message in"
    );
}

#[tokio::test]
async fn trash_takes_the_route_a_person_s_trash_takes() {
    let local = local();
    let trash = test_support::mailbox(&local.connection, &local.account, "Trash");
    let message = arrive(&local, &compile(&[rule("bin", &["trash"])])).await;

    assert_eq!(
        message.mailbox_id, trash.id,
        "`trash` moves the message to the account's Trash, resolved by role"
    );
    // The acceptance criterion, and the reason it is asserted on the queued
    // operation rather than on a "was it recoverable" test: `Delete { from,
    // trash }` is what `postio_session::actions` writes when a person presses
    // the key, and it is recoverable because moving it back out is an
    // ordinary move. A rule that enqueued `Move` instead would look identical
    // locally and be a different thing on the server.
    assert!(
        queued(&local).iter().any(|operation| matches!(
            operation,
            Operation::Delete { trash: to, .. } if *to == trash.id
        )),
        "a rule's trash has to be the same operation a person's trash is -- \
         never an expunge, and never a plain move the server would not \
         recognise as trashing"
    );
    assert!(
        !queued(&local)
            .iter()
            .any(|operation| matches!(operation, Operation::Expunge { .. })),
        "and nothing a rule does may be a permanent delete (ADR 0008 Q5)"
    );
}

#[tokio::test]
async fn archive_resolves_the_role_rather_than_a_folder_called_archive() {
    let local = local();
    let archive = test_support::mailbox(&local.connection, &local.account, "Archive");
    let message = arrive(&local, &compile(&[rule("keep", &["archive"])])).await;

    assert_eq!(
        message.mailbox_id, archive.id,
        "`archive` is the account's Archive by role, which is the same \
         resolution the interactive verb does"
    );
}

#[tokio::test]
async fn stop_halts_the_rules_below_it_on_that_message() {
    let local = local();
    let message = arrive(
        &local,
        &compile(&[stopping("first", &["flag"]), rule("second", &["mark-read"])]),
    )
    .await;

    assert!(
        message.flags.contains(&Flag::Flagged),
        "the stopping rule's own actions still run -- `stop` halts what comes \
         *after* it, it does not cancel the rule carrying it"
    );
    assert!(
        !message.flags.contains(&Flag::Seen),
        "`stop` has to prevent the rules below it from being evaluated at \
         all: that is the whole of ADR 0008 Q4, and a rule set whose order \
         does not matter is a rule set nobody can reason about"
    );
}

#[tokio::test]
async fn stop_halts_that_message_only_and_not_the_pass() {
    let local = local();
    // Two messages, and the first one stops. The second must still be
    // evaluated from the top: `stop` is scoped to the message being filed,
    // not to the pass filing it, and a `stop` that leaked across messages
    // would silently disable every rule for the rest of a sync.
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(MockMessage::new(note()).with_internal_date(at(1)))
        .message(
            MockMessage::new(
                "From: Ada Lovelace <ada@example.com>\r\n\
                 Subject: Note two\r\n\
                 Message-ID: <note-2@example.com>\r\n\
                 Content-Type: text/plain; charset=utf-8\r\n\
                 \r\n\
                 Another short note.\r\n"
                    .to_string()
                    .into_bytes(),
            )
            .with_internal_date(at(2)),
        );
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");

    let report = sync_mailbox_with_rules(
        &local.connection,
        &backend,
        &local.inbox,
        &CancelToken::new(),
        &compile(&[stopping("first", &["flag"])]),
        |_| {},
    )
    .await
    .expect("headers");
    assert_eq!(report.inserted, 2, "both fixture messages have to land");

    let messages = MessageRepository::new(&local.connection);
    for uid in [1u32, 2] {
        let message = messages
            .by_uid(
                local.inbox.id,
                postio_model::Generation::new(VALIDITY),
                Uid::new(uid),
            )
            .expect("look up")
            .unwrap_or_else(|| panic!("message {uid} is stored"));
        assert!(
            message.flags.contains(&Flag::Flagged),
            "message {uid} must have been evaluated from the top of the rule \
             list: a `stop` on an earlier message has nothing to say about \
             this one"
        );
    }
}

#[tokio::test]
async fn label_writes_the_join_the_keyword_and_the_queue() {
    let local = local();
    let label = a_label(&local, "Invoices");
    let message = arrive(&local, &compile(&[rule("tag", &["label:Invoices"])])).await;

    assert_eq!(
        LabelRepository::new(&local.connection)
            .for_message(message.id)
            .expect("read the labels"),
        vec![label.id],
        "`label:` has to reach the join row -- that is what the list and the \
         reader draw a label from"
    );
    assert!(
        message
            .flags
            .contains(&Flag::Keyword("Invoices".to_owned())),
        "and the keyword, which is how the label travels to the server: a \
         join row alone is a label no other client ever sees. Got {:?}",
        message.flags
    );
    assert!(
        queued(&local).iter().any(|operation| matches!(
            operation,
            Operation::SetFlags { flags } if flags.contains(&Flag::Keyword("Invoices".to_owned()))
        )),
        "and the queue row, or the next resync takes the keyword back off"
    );
}

#[tokio::test]
async fn a_label_naming_one_that_does_not_exist_leaves_the_mail_alone() {
    let local = local();
    let message = arrive(&local, &compile(&[rule("tag", &["label:Nowhere", "flag"])])).await;

    assert!(
        LabelRepository::new(&local.connection)
            .for_message(message.id)
            .expect("read the labels")
            .is_empty(),
        "an unresolvable label must not put some other label on the message"
    );
    assert!(
        message.flags.contains(&Flag::Flagged),
        "and it must not stop the actions after it, for the same reason an \
         unresolvable `move:` does not: ADR 0008 Q6 is that an error never \
         drops mail"
    );
}

#[tokio::test]
async fn a_label_the_message_already_carries_is_not_queued_again() {
    let local = local();
    a_label(&local, "Invoices");
    let rules = compile(&[rule("tag", &["label:Invoices", "label:Invoices"])]);
    arrive(&local, &rules).await;

    let keyword_writes = queued(&local)
        .into_iter()
        .filter(|operation| matches!(operation, Operation::SetFlags { .. }))
        .count();
    assert_eq!(
        keyword_writes, 1,
        "the second `label:` finds the label already on the message and has \
         nothing to do: a rule that fires on every arrival would otherwise \
         queue a redundant SetFlags per message, which is the same filter \
         `flag` applies"
    );
}

/// A label the account owns, created the way the picker creates one.
fn a_label(local: &Local, name: &str) -> Label {
    let mut label = Label::new(local.account.id, name);
    LabelRepository::new(&local.connection)
        .create(&mut label)
        .expect("create a label");
    label
}
