//! Carrying out what a rule decided (#481, ADR 0028).
//!
//! `postio_search::rules` says *which* rules select a message and at which of
//! the two evaluation points; this runs their actions. It is the second half
//! of ADR 0008 Q5, and its whole design is one sentence from that Q:
//!
//! > Every action is local-first, exactly like a keystroke: SQLite write,
//! > enqueue the remote operation, emit the event. **There is no rules-only
//! > mutation path.**
//!
//! So nothing here writes a row or builds an [`Operation`](postio_model::Operation).
//! Every action reaches [`postio_storage::actions`] — the same verb the
//! command bus calls when a person presses `a` — which is what makes `trash`
//! recoverable the same way a hand-trashed message is, by construction rather
//! than by a test that happens to agree. `forward:` is the same sentence one
//! step further out: it goes through the draft and the queue a person's send
//! goes through, so a forwarded message is in Sent and there is no second
//! outbound path.
//!
//! # What a rule does not do
//!
//! Two of the three things [`postio_session::actions`] does, deliberately
//! (ADR 0028 Q1):
//!
//! * **No undo entry.** Undo walks back through the *user's* history, and a
//!   rule firing inside a sync is not in it; a `u` that reverses something
//!   the user never saw is worse than no undo. Nothing is lost — ADR 0008 Q5
//!   refuses `delete` to a rule, so every effect here is reversible by the
//!   ordinary verb on the ordinary message.
//! * **No event of its own.** The caller owns the transaction and emits once
//!   it commits. ADR 0008 Q3 puts a header rule's action *before* any event
//!   is emitted, so the user never sees the mail land in the Inbox and jump.
//!
//! # The transaction is the caller's
//!
//! Every function takes a `&Transaction` it did not open, because ADR 0008 Q3
//! requires a header rule's action to be in the same transaction as the
//! insert that brought the message in. A verb that opened its own could not
//! be called from here at all.
//!
//! # Both evaluation points
//!
//! The arrival point calls this inside the insert transaction (ADR 0008 Q3);
//! the body point calls it inside a transaction it opens for the purpose,
//! because by then the insert is long committed. ADR 0030 is why the second
//! one has to exist at all: `forward:` stages on the body whatever its query
//! says, so a body point that reported and did nothing would be a rule that
//! never runs.
//!
//! # Not here yet
//!
//! * Per-rule error isolation and Attention are #483. What this module does
//!   with an action it cannot carry out — a `move:` naming a mailbox that
//!   does not exist — is leave the message alone and carry on, so the rules
//!   after it still run and the mail is never dropped (ADR 0008 Q6). Saying
//!   so out loud, where the user can see it, is #483's.

use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use std::collections::BTreeMap;

use postio_model::mailbox::MailboxRole;
use postio_model::rule::{Action, Rule};
use postio_model::{AccountId, EmailAddress, Flag, Message};
use postio_storage::actions::{self, Relocation};
use postio_storage::repository::{
    AccountRepository, DraftRepository, LabelRepository, MailboxRepository, MessageRepository,
    RuleForwardRepository,
};

use crate::initial::Result;

/// Runs the actions of `rules`, in order, over `message`.
///
/// `message` is updated as it goes, so an action reads what the one before it
/// wrote: `["flag", "mark-read"]` has to set both, and `set_flag` computes the
/// new flag set from the row it is handed. A stale copy would quietly undo
/// the earlier action.
///
/// # Rules rather than a flat list of actions
///
/// It used to take the actions alone, which is all any of them needed while
/// none of them left the machine. `forward:` needs to know **which rule**
/// asked: its rate cap is per rule per hour (ADR 0008 Q5), and the record it
/// writes is what a person reads to find out why a message they never wrote
/// is in their Sent folder. Flattening first threw that away.
pub(crate) fn apply(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &mut Message,
    rules: &[&Rule],
    at: DateTime<Utc>,
) -> Result<()> {
    for rule in rules {
        apply_one(transaction, account, message, rule, at)?;
    }
    Ok(())
}

/// One rule's actions, in order. See [`apply`].
fn apply_one(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &mut Message,
    rule: &Rule,
    at: DateTime<Utc>,
) -> Result<()> {
    for action in &rule.actions {
        match action {
            Action::Flag => flag(transaction, account, message, Flag::Flagged, true, at)?,
            Action::Unflag => flag(transaction, account, message, Flag::Flagged, false, at)?,
            Action::MarkRead => flag(transaction, account, message, Flag::Seen, true, at)?,
            Action::MarkUnread => flag(transaction, account, message, Flag::Seen, false, at)?,
            Action::Move(path) => {
                let destination = MailboxRepository::new(transaction)
                    .by_path(account, path)?
                    .map(|mailbox| mailbox.id);
                relocate(
                    transaction,
                    account,
                    message,
                    destination,
                    Relocation::Move,
                    at,
                )?;
            }
            Action::Archive => {
                let destination = by_role(transaction, account, MailboxRole::Archive)?;
                relocate(
                    transaction,
                    account,
                    message,
                    destination,
                    Relocation::Move,
                    at,
                )?;
            }
            // `Relocation::Trash`, which is what makes the server see
            // `Operation::Delete { from, trash }` rather than a plain move --
            // the same row a person's `trash` writes, and the reason this is
            // recoverable without a recovery path of its own.
            Action::Trash => {
                let destination = by_role(transaction, account, MailboxRole::Trash)?;
                relocate(
                    transaction,
                    account,
                    message,
                    destination,
                    Relocation::Trash,
                    at,
                )?;
            }
            Action::Label(name) => label(transaction, account, message, name, at)?,
            Action::Forward(target) => {
                forward(transaction, account, message, &rule.name, target, at)?;
            }
        }
    }
    Ok(())
}

/// Set or clear one flag, and keep the in-memory copy honest for the next
/// action in the list.
fn flag(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &mut Message,
    flag: Flag,
    wanted: bool,
    at: DateTime<Utc>,
) -> Result<()> {
    // The verb writes what it is given, so a row already in the wanted state
    // is the caller's to filter -- and filtering it is what keeps a rule that
    // fires on every arrival from queueing a redundant `SetFlags` per message.
    if message.flags.contains(&flag) == wanted {
        return Ok(());
    }
    actions::set_flag(transaction, account, &[&*message], &flag, wanted, at)?;
    if wanted {
        message.flags.insert(flag);
    } else {
        message.flags.remove(&flag);
    }
    Ok(())
}

/// Put the label called `name` on `message`, and keep the in-memory copy
/// honest for the next action in the list.
///
/// A label a rule names but the account has not got leaves the message alone
/// and lets the rules after it run — the same answer an unresolvable `move:`
/// gets, and for the same reason (ADR 0008 Q6). Creating one here instead
/// would be a rule inventing a row in the user's own label set, which is a
/// product decision and not this issue's (#1150).
fn label(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &mut Message,
    name: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    let labels = LabelRepository::new(transaction);
    let Some(label) = labels.by_name(account, name)? else {
        return Ok(());
    };
    // The verb writes what it is given, exactly as `set_flag` does, so a
    // message already carrying the label is filtered here -- otherwise a rule
    // that fires on every arrival queues a redundant keyword write per
    // message. The join row is what is asked, not the keyword: the two are
    // written together and the join is the one a resync cannot disturb.
    if labels.for_message(message.id)?.contains(&label.id) {
        return Ok(());
    }
    actions::set_label(transaction, account, &[&*message], &label, true, at)?;
    message.flags.insert(Flag::Keyword(label.name));
    Ok(())
}

fn by_role(
    transaction: &Transaction<'_>,
    account: AccountId,
    role: MailboxRole,
) -> Result<Option<postio_model::MailboxId>> {
    Ok(MailboxRepository::new(transaction)
        .by_role(account, role)?
        .map(|mailbox| mailbox.id))
}

/// Move `message` to `destination`, and keep the in-memory copy honest.
///
/// `None` is a destination that does not exist — a `move:` naming a mailbox
/// the account has not got, or an `archive` on an account with no Archive
/// folder. The message stays where it is and the rules after this one still
/// run: ADR 0008 Q6 is that an error never drops mail, and the loudest thing
/// this could do instead — failing the pass — would roll back the insert that
/// brought the message in. Telling the user is #483's.
fn relocate(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &mut Message,
    destination: Option<postio_model::MailboxId>,
    relocation: Relocation,
    at: DateTime<Utc>,
) -> Result<()> {
    let Some(destination) = destination else {
        return Ok(());
    };
    if destination == message.mailbox_id {
        return Ok(());
    }
    let by_source = BTreeMap::from([(message.mailbox_id, vec![message.id])]);
    actions::relocate(
        transaction,
        account,
        &by_source,
        destination,
        relocation,
        at,
    )?;
    message.mailbox_id = destination;
    Ok(())
}

/// How many messages one rule may forward in an hour.
///
/// ADR 0008 Q5 asks for a cap and does not name a number. This one is chosen
/// against what it is defending: a rule matching a busy mailing list forwards
/// a few dozen messages on a heavy day, and a rule that has started forwarding
/// *everything* — a query that matches more than its author thought, or mail
/// looping back through an address that delivers here — reaches this inside a
/// minute. Low enough to stop the second before it is a bill, high enough that
/// the first never notices.
///
/// Hitting it never drops mail. The message stays where it is and the rules
/// after it still run (ADR 0008 Q6); what does not happen is the send.
const FORWARDS_PER_HOUR: u32 = 50;

/// Send `message` on to `target`, as the rule called `rule` asked.
///
/// The one action that leaves the machine, and the only one with guards. All
/// three refuse the *send* and nothing else: the message stays where it is,
/// the rest of the rule runs, and no mail is dropped (ADR 0008 Q6).
///
/// 1. **Never forward what a rule already forwarded.** Checked against the
///    marker [`FORWARDED_BY_A_RULE`](postio_model::outgoing::FORWARDED_BY_A_RULE)
///    puts on the way out. This is the loop: forwarding to an address that
///    delivers back into this account arrives as a *new* message, which no
///    local record recognises and the header does. It is answerable here
///    because the header block arrives with the body (ADR 0025 Q4) and
///    `forward:` waits for the body anyway (ADR 0030).
/// 2. **Never forward to one of the user's own addresses**, across every
///    configured account rather than only this one — the loop above, arranged
///    by hand. And the target is a literal, always: it comes from the parsed
///    action and nothing about the message is ever substituted into it, so a
///    message cannot choose where it is sent.
/// 3. **Never more than [`FORWARDS_PER_HOUR`] for one rule.**
///
/// The send itself is a draft and an `Operation::Send`, exactly as a person's
/// send is (ADR 0028): so a forwarded message appears in Sent, retries and
/// backs off the way everything else does, and there is no second outbound
/// path to teach separately about metered connections.
fn forward(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &Message,
    rule: &str,
    target: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    // Nothing to forward. `forward:` stages on the body precisely so this
    // cannot happen (ADR 0030), and it is checked anyway: a forward of a
    // message with no content is the failure the staging rule exists to
    // prevent, and it must not become possible again by some other caller.
    if message.body.text.is_none() && message.body.html.is_none() {
        tracing::warn!(
            rule,
            message = message.id.get(),
            "a forward was reached with no body and did not send"
        );
        return Ok(());
    }

    let accounts = AccountRepository::new(transaction);
    let Some(owner) = accounts.get(account)? else {
        return Ok(());
    };
    let destination = EmailAddress::new(None::<&str>, target);

    // Guard 2, over every account rather than this one: two accounts in one
    // Postio forwarding to each other is the same loop with more steps.
    if accounts
        .list()?
        .iter()
        .any(|other| other.owns_address(&destination))
    {
        tracing::info!(
            rule,
            "a forward: names an address of a configured account and did not send"
        );
        return Ok(());
    }

    // Guard 1.
    if MessageRepository::new(transaction)
        .headers(message.id)?
        .is_some_and(|headers| headers.contains(postio_model::outgoing::FORWARDED_BY_A_RULE))
    {
        tracing::info!(
            rule,
            message = message.id.get(),
            "a rule had already forwarded this message, so it was not forwarded again"
        );
        return Ok(());
    }

    // Guard 3.
    let forwards = RuleForwardRepository::new(transaction);
    let since = at - chrono::TimeDelta::hours(1);
    if forwards.count_since(account, rule, since)? >= FORWARDS_PER_HOUR {
        // #483 is what turns this into something the user is shown. Until
        // then it is a log line, and the mail is untouched either way, which
        // is the half ADR 0008 Q6 actually requires.
        tracing::warn!(
            rule,
            cap = FORWARDS_PER_HOUR,
            "a rule reached its forwarding cap for the hour and did not send"
        );
        return Ok(());
    }

    let mut draft =
        postio_model::reply::forward(message, &owner, postio_model::reply::plain_forward(message));
    draft.to = vec![destination];
    draft.forwarded_by = Some(rule.to_owned());
    // Straight to the queue: a draft nobody is going to open a composer on.
    // `queue_send` is the same call the composer makes, so this reserves the
    // same `Message-ID`, writes the same row and settles the same way.
    DraftRepository::new(transaction).queue_send(&mut draft, at)?;
    forwards.record(account, rule, message.id, at)?;
    Ok(())
}
