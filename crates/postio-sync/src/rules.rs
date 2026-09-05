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
//! than by a test that happens to agree.
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
//! # Not here yet
//!
//! * `label:` — its mutating half has not been lifted into `postio-storage`
//!   the way `relocate` and `set_flag` were (#1125), and re-implementing it
//!   here would be exactly the rules-only mutation path the ADR forbids
//!   (#1141).
//! * `forward:` — needs a body an on-arrival rule has not fetched, which is a
//!   staging question ADR 0028 does not answer (#1142).
//! * Per-rule error isolation and Attention are #483. What this module does
//!   with an action it cannot carry out — a `move:` naming a mailbox that
//!   does not exist — is leave the message alone and carry on, so the rules
//!   after it still run and the mail is never dropped (ADR 0008 Q6). Saying
//!   so out loud, where the user can see it, is #483's.

use chrono::{DateTime, Utc};
use rusqlite::Transaction;
use std::collections::BTreeMap;

use postio_model::mailbox::MailboxRole;
use postio_model::rule::Action;
use postio_model::{AccountId, Flag, Message};
use postio_storage::actions::{self, Relocation};
use postio_storage::repository::MailboxRepository;

use crate::initial::Result;

/// Runs `actions`, in order, over `message`.
///
/// `message` is updated as it goes, so an action reads what the one before it
/// wrote: `["flag", "mark-read"]` has to set both, and `set_flag` computes the
/// new flag set from the row it is handed. A stale copy would quietly undo
/// the earlier action.
pub(crate) fn apply(
    transaction: &Transaction<'_>,
    account: AccountId,
    message: &mut Message,
    actions_to_run: &[Action],
    at: DateTime<Utc>,
) -> Result<()> {
    for action in actions_to_run {
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
            // See the module docs: both are filed, and doing half of either
            // here is worse than not doing it.
            Action::Label(_) | Action::Forward(_) => {}
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
