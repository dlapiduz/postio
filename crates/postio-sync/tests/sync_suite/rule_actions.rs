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

/// A server holding one message of the caller's choosing.
///
/// The guards below turn on what is *in* the message — a header a previous
/// forward left on it — so they cannot all share one fixture.
async fn server_of(raw: Vec<u8>) -> MockBackend {
    let inbox = MockMailbox::new(INBOX)
        .uid_validity(UidValidity::new(VALIDITY))
        .message(MockMessage::new(raw).with_internal_date(at(1)));
    let backend = MockBackend::builder().mailbox(inbox).build();
    backend.connect().await.expect("connect");
    backend
}

/// The fixture message as a rule would see it if a rule had already forwarded
/// it — the loop, arriving back here as new mail.
fn already_forwarded() -> Vec<u8> {
    "From: Ada Lovelace <ada@example.com>\r\n\
     Subject: Note one\r\n\
     Message-ID: <note-1@example.com>\r\n\
     X-Postio-Forwarded: 1\r\n\
     Content-Type: text/plain; charset=utf-8\r\n\
     \r\n\
     A short note.\r\n"
        .to_string()
        .into_bytes()
}

struct Local {
    #[allow(dead_code)]
    database: TempDatabase,
    connection: postio_storage::PooledConnection,
    blobs: postio_storage::BlobStore,
    account: Account,
    inbox: Mailbox,
}

fn local() -> Local {
    let database = test_support::temp();
    let connection = database.connection().expect("checkout");
    let account = test_support::account(&connection);
    let inbox = test_support::mailbox(&connection, &account, INBOX);
    let blobs = postio_storage::BlobStore::open(
        database.directory().join("blobs"),
        &postio_storage::test_support::blob_keys(),
    )
    .expect("a blob store");
    Local {
        database,
        connection,
        blobs,
        account,
        inbox,
    }
}

/// Fetch the fixture message's body, running the rules that were waiting for
/// it — the second of the two evaluation points.
async fn body_lands(local: &Local, rules: &RuleSet) -> postio_sync::backfill::BodyFetch {
    body_lands_of(local, rules, note()).await
}

async fn body_lands_of(
    local: &Local,
    rules: &RuleSet,
    raw: Vec<u8>,
) -> postio_sync::backfill::BodyFetch {
    let size = raw.len() as u64;
    let backend = server_of(raw).await;
    let message = stored(local);
    postio_sync::backfill::fetch_body_with_rules(
        &local.connection,
        &local.blobs,
        &backend,
        &postio_sync::backfill::BodyRequest {
            message: message.id,
            mailbox: local.inbox.id,
            path: local.inbox.path.clone(),
            uid: Uid::new(1),
            remote_id: postio_model::RemoteId::new(format!("{VALIDITY}:1")),
            size,
            received_at: at(1),
            want: postio_sync::backfill::Want::Text,
        },
        None,
        rules,
        &CancelToken::new(),
    )
    .await
    .expect("the body arrives")
}

/// The rule names a [`body_lands`] fetch reported.
fn names(fetch: &postio_sync::backfill::BodyFetch) -> Vec<&str> {
    fetch.fired.iter().map(|hit| hit.rule.as_str()).collect()
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
    arrive_of(local, rules, note()).await
}

async fn arrive_of(local: &Local, rules: &RuleSet, raw: Vec<u8>) -> postio_model::Message {
    let backend = server_of(raw).await;
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

/// A rule that waited for the body carries its actions out when it arrives
/// (ADR 0030).
///
/// The arrival point has run actions since #481; the body point only ever
/// *reported* which rules matched. That gap is invisible for as long as every
/// body-staged rule is one somebody wrote to file mail on its contents — the
/// report goes to the log and the mail sits where it was. ADR 0030 makes it
/// load-bearing: `forward:` stages here whatever its query says, so a body
/// point that reports and does not act is a rule that never runs.
#[tokio::test]
async fn a_rule_that_waited_for_the_body_carries_its_actions_out() {
    let local = local();
    let rules = compile(&[rule_matching("bodies", "body:short", &["flag"])]);

    let message = arrive(&local, &rules).await;
    assert!(
        !message.flags.contains(&Flag::Flagged),
        "the body rule cannot have run on arrival -- its body was not local, \
         so this test would prove nothing about the body point"
    );

    let fetch = body_lands(&local, &rules).await;
    assert_eq!(names(&fetch), vec!["bodies"], "the rule has to match");

    assert!(
        stored(&local).flags.contains(&Flag::Flagged),
        "the body landed, the rule matched, and nothing carried its actions \
         out: a rule staged on the body reports and does nothing"
    );
}

/// And through the same verbs, so the server hears about it too.
#[tokio::test]
async fn a_body_staged_action_enqueues_the_operation_the_server_needs() {
    let local = local();
    let rules = compile(&[rule_matching("bodies", "body:short", &["flag"])]);
    arrive(&local, &rules).await;
    body_lands(&local, &rules).await;

    let flag_writes = queued(&local)
        .into_iter()
        .filter(|operation| matches!(operation, Operation::SetFlags { .. }))
        .count();
    assert_eq!(
        flag_writes, 1,
        "local-first is both halves at the body point as well: the row and \
         the queue row (ARCHITECTURE.md §1)"
    );
}

/// A body-staged rule that moves mail says which mailbox it left (#1142).
///
/// Nothing downstream can emit a precise event otherwise. The arrival point
/// never had this problem — its actions run inside the sync pass, which
/// announces the mailbox it just synced — while a body fetch announces only
/// its own progress, so a rule that filed a message at this point would move
/// it out from under a list that never heard.
#[tokio::test]
async fn a_body_staged_move_says_which_mailbox_the_message_left() {
    let local = local();
    let lists = test_support::mailbox(&local.connection, &local.account, "Lists");
    let rules = compile(&[rule_matching("bodies", "body:short", &["move:Lists"])]);

    arrive(&local, &rules).await;
    let fetch = body_lands(&local, &rules).await;

    assert_eq!(names(&fetch), vec!["bodies"], "the rule has to match");
    assert_eq!(
        fetch.relocated,
        Some(postio_sync::backfill::Relocated {
            from: local.inbox.id,
            to: lists.id,
        }),
        "the message was filed into another folder and the fetch reported \
         nothing about it, so both lists are stale until something else \
         happens to reload them"
    );
    assert_eq!(
        stored(&local).mailbox_id,
        lists.id,
        "the move has to have actually happened"
    );
}

/// A body-staged rule that changes nothing about where the message lives
/// reports no relocation, so the blunt event is not emitted for a flag.
#[tokio::test]
async fn a_body_staged_flag_is_not_a_relocation() {
    let local = local();
    let rules = compile(&[rule_matching("bodies", "body:short", &["flag"])]);
    arrive(&local, &rules).await;

    assert_eq!(
        body_lands(&local, &rules).await.relocated,
        None,
        "a flag leaves the message exactly where it was: reporting a move \
         here would cost every list a full reload per flagged body"
    );
}

/// A `forward:` rule sends the message on, through the draft path (ADR 0028).
///
/// ADR 0008 Q5's last action, and the only one that leaves the machine. It
/// runs at the body point (ADR 0030) because it needs the message it is
/// forwarding, and it goes out the way a person's send does — a draft, and an
/// `Operation::Send` on the queue — so a forwarded message appears in Sent
/// like any other and there is no second send path to teach about metered
/// connections and retries.
#[tokio::test]
async fn a_forwarding_rule_queues_a_send_when_the_body_lands() {
    let local = local();
    let rules = compile(&[rule("digest", &["forward:babbage@example.com"])]);

    arrive(&local, &rules).await;
    assert!(
        !queued(&local)
            .iter()
            .any(|operation| matches!(operation, Operation::Send { .. })),
        "the forward cannot have gone out on arrival: its body was not local, \
         so it would have sent headers with nothing under them"
    );

    let fetch = body_lands(&local, &rules).await;
    assert_eq!(names(&fetch), vec!["digest"], "the rule has to match");

    let sends: Vec<Operation> = queued(&local)
        .into_iter()
        .filter(|operation| matches!(operation, Operation::Send { .. }))
        .collect();
    assert_eq!(
        sends.len(),
        1,
        "a `forward:` rule matched and nothing was queued to send: {sends:?}"
    );

    let Operation::Send { draft } = sends[0] else {
        unreachable!("filtered to sends")
    };
    let draft = postio_storage::repository::DraftRepository::new(&local.connection)
        .get(draft)
        .expect("read the draft")
        .expect("the queued send names a draft that exists");
    assert_eq!(
        draft
            .to
            .iter()
            .map(|to| to.address.as_str())
            .collect::<Vec<_>>(),
        vec!["babbage@example.com"],
        "the forward went somewhere other than where the rule said"
    );
    assert_eq!(
        draft.state,
        postio_model::draft::DraftState::Queued,
        "a draft a rule made is handed straight to the queue: nobody is going \
         to open a composer on it"
    );
    assert!(
        draft
            .body
            .text
            .as_deref()
            .is_some_and(|text| text.contains("A short note")),
        "the forwarded message carries the original: {:?}",
        draft.body.text
    );
}

/// The count of `Operation::Send` rows on the queue.
fn sends(local: &Local) -> usize {
    queued(local)
        .into_iter()
        .filter(|operation| matches!(operation, Operation::Send { .. }))
        .count()
}

/// Guard one: never forward a message a rule has already forwarded.
///
/// The loop. A rule forwarding to an address that delivers back into this
/// account gets the mail again as a *new* message — new local id, new UID —
/// so nothing local recognises it and it would be forwarded again, for ever.
/// The marker travels with the message, which is the only thing that can.
///
/// Answerable at the body point because the header block arrives with the
/// body (ADR 0025 Q4), which is where `forward:` runs anyway (ADR 0030).
#[tokio::test]
async fn a_message_a_rule_already_forwarded_is_not_forwarded_again() {
    let local = local();
    let rules = compile(&[rule("digest", &["forward:babbage@example.com"])]);

    arrive_of(&local, &rules, already_forwarded()).await;
    let fetch = body_lands_of(&local, &rules, already_forwarded()).await;

    assert_eq!(
        names(&fetch),
        vec!["digest"],
        "the rule still matches -- the guard refuses the send, not the match"
    );
    assert_eq!(
        sends(&local),
        0,
        "a message carrying Postio's own forwarding marker was forwarded \
         again: a rule pointed at an address that delivers back here would \
         forward the same mail round and round"
    );
}

/// Guard two: never forward to an address of a configured account.
///
/// The same loop, arranged by hand, and the one a person is most likely to
/// write by accident — a rule that forwards "to me".
#[tokio::test]
async fn a_forward_to_one_of_the_users_own_addresses_does_not_send() {
    let local = local();
    let own = local.account.address.address.clone();
    let rules = compile(&[rule("digest", &[&format!("forward:{own}")])]);

    arrive(&local, &rules).await;
    let fetch = body_lands(&local, &rules).await;

    assert_eq!(names(&fetch), vec!["digest"], "the rule still matches");
    assert_eq!(
        sends(&local),
        0,
        "a rule forwarding to {own}, which is this account's own address, was \
         allowed to send: every message it matches arrives again and matches \
         again"
    );
}

/// Guard two's other half: the target is a literal, and nothing from the
/// message is ever substituted into it.
///
/// There is no interpolation syntax, and this is what says so out loud: a
/// target that looks like a placeholder is an address, verbatim. A rule
/// language that grew one would let the *message* choose where a copy of
/// itself is sent, which is the one thing a forwarding guard cannot survive.
#[tokio::test]
async fn a_forward_target_is_never_built_from_the_message() {
    let local = local();
    let rules = compile(&[rule("digest", &["forward:${from}@example.net"])]);

    arrive(&local, &rules).await;
    body_lands(&local, &rules).await;

    let queue = queued(&local);
    let Some(Operation::Send { draft }) = queue
        .iter()
        .find(|operation| matches!(operation, Operation::Send { .. }))
    else {
        panic!("the forward should still send: {queue:?}");
    };
    let draft = *draft;
    let draft = postio_storage::repository::DraftRepository::new(&local.connection)
        .get(draft)
        .expect("read the draft")
        .expect("the queued send names a draft");
    assert_eq!(
        draft
            .to
            .iter()
            .map(|to| to.address.as_str())
            .collect::<Vec<_>>(),
        vec!["${from}@example.net"],
        "something in the target was replaced with a value out of the \
         message, so the message chose where it was sent"
    );
}

/// Guard three: a rule may not forward more than its hourly cap.
///
/// What stops a rule that has started forwarding everything — a query that
/// matches more than its author thought, or mail arriving in a loop the first
/// two guards cannot see. It refuses the *send*: the message is untouched and
/// the rules after it still run, because ADR 0008 Q6 is that nothing a rule
/// does may drop mail.
#[tokio::test]
async fn a_rule_that_has_hit_its_hourly_cap_stops_forwarding() {
    let local = local();
    let rules = compile(&[rule("digest", &["forward:babbage@example.com"])]);
    arrive(&local, &rules).await;

    // The hour this rule has already had. Written straight to the log rather
    // than by forwarding fifty messages, which would be a test of the fixture.
    let forwards = postio_storage::repository::RuleForwardRepository::new(&local.connection);
    for n in 0..50 {
        forwards
            .record(
                local.account.id,
                "digest",
                postio_model::MessageId::new(1_000 + n),
                // Against the wall clock, because the cap is: the body point
                // stamps its actions with `Utc::now()`, so an hour ago means
                // an hour before this test ran and not before the fixture's
                // own 2026-03-01.
                Utc::now() - chrono::TimeDelta::minutes(30),
            )
            .expect("record a forward");
    }

    let fetch = body_lands(&local, &rules).await;

    assert_eq!(names(&fetch), vec!["digest"], "the rule still matches");
    assert_eq!(
        sends(&local),
        0,
        "the rule had already forwarded its hour's worth and forwarded again"
    );
    assert_eq!(
        stored(&local).mailbox_id,
        local.inbox.id,
        "the message was dropped or moved when the cap was hit; hitting a cap \
         must leave the mail exactly where it was"
    );
}

/// And the cap counts the rule, not the account: a second rule has its own.
#[tokio::test]
async fn one_rules_cap_does_not_silence_another() {
    let local = local();
    let rules = compile(&[
        rule("noisy", &["forward:babbage@example.com"]),
        rule("quiet", &["forward:hopper@example.com"]),
    ]);
    arrive(&local, &rules).await;

    let forwards = postio_storage::repository::RuleForwardRepository::new(&local.connection);
    for n in 0..50 {
        forwards
            .record(
                local.account.id,
                "noisy",
                postio_model::MessageId::new(1_000 + n),
                // Against the wall clock, because the cap is: the body point
                // stamps its actions with `Utc::now()`, so an hour ago means
                // an hour before this test ran and not before the fixture's
                // own 2026-03-01.
                Utc::now() - chrono::TimeDelta::minutes(30),
            )
            .expect("record a forward");
    }

    body_lands(&local, &rules).await;

    assert_eq!(
        sends(&local),
        1,
        "the second rule stopped sending because the first had used up its \
         own hour: a cap that is really per account punishes the rules that \
         behaved"
    );
}
