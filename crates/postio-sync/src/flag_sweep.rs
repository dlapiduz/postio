//! Reading every flag in a mailbox, for a server whose change feed cannot be
//! believed.
//!
//! The decision to sweep is [`postio_model::SweepPolicy::decide`]'s; this is
//! the doing of it. The server's whole flag listing
//! ([`MailBackend::fetch_flags`]) is compared with the store's, and only the
//! messages that disagree are written, through
//! [`MessageRepository::apply_server_flags`], which keeps what the user has
//! done that the server has not yet heard.
//!
//! A sweep only ever *corrects flags*. A message the server lists and the
//! store lacks is an arrival, which the `UIDNEXT` witness in [`crate::resync`]
//! already owns; a message the store holds and the server omits is a deletion,
//! which the count arithmetic there owns. Doing either here as well would be
//! two answers to one question.

use std::collections::BTreeMap;

use postio_account::backend::MailBackend;
use postio_account::cancel::CancelToken;
use postio_model::{FlagSet, Generation, Mailbox, MessageId, Uid};
use postio_storage::Checkout;
use postio_storage::repository::MessageRepository;

use crate::drain::SyncError;
use crate::initial;

/// What one sweep did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Swept {
    /// Whether every flag was actually read. `false` when the backend cannot
    /// list flags alone, so the pass is not counted as having swept.
    pub swept: bool,
    /// Messages whose stored flags the sweep changed.
    pub corrected: usize,
}

/// The messages whose stored flags differ from what the server lists.
///
/// Pure, so the comparison is red in a second. `\Recent` is compared as the
/// store would keep it (not at all): a server reporting it on one pass and
/// not the next is not a change anyone made.
pub(crate) fn disagreements(
    stored: &[(MessageId, Uid, FlagSet)],
    server: Vec<(Uid, FlagSet)>,
) -> Vec<(MessageId, FlagSet)> {
    let by_uid: BTreeMap<Uid, (MessageId, &FlagSet)> = stored
        .iter()
        .map(|(id, uid, flags)| (*uid, (*id, flags)))
        .collect();
    server
        .into_iter()
        .filter_map(|(uid, flags)| {
            let (id, held) = by_uid.get(&uid)?;
            (held.persistable() != flags.persistable()).then_some((*id, flags))
        })
        .collect()
}

/// Reads `mailbox`'s flags from the server and writes the ones that moved.
///
/// Errors propagate, and the caller must not record the server's
/// `HIGHESTMODSEQ` until this has returned: advancing it past a pass whose
/// flags were not reconciled is the way a missed change becomes permanent.
pub(crate) async fn sweep(
    connection: &Checkout,
    backend: &dyn MailBackend,
    mailbox: &Mailbox,
    generation: Generation,
    cancel: &CancelToken,
) -> Result<Swept, SyncError> {
    let Some(server) = backend.fetch_flags(&mailbox.path, cancel).await? else {
        tracing::debug!(
            mailbox = mailbox.id.get(),
            "the backend cannot list flags alone; trusting its change feed"
        );
        return Ok(Swept::default());
    };

    let messages = MessageRepository::new(connection);
    let stored = messages.flags_in(mailbox.id, generation).await?;
    let differing = disagreements(&stored, server);

    let mut corrected = 0;
    let mut rest: &[(MessageId, FlagSet)] = &differing;
    while !rest.is_empty() {
        let take = initial::unit_rows().min(rest.len());
        let (unit, tail) = rest.split_at(take);
        rest = tail;
        let (changed, held) = initial::write_unit(connection, || async {
            Ok(MessageRepository::new(connection)
                .apply_server_flags(unit)
                .await?)
        })
        .await?;
        corrected += changed.len();
        initial::unit_wrote(unit.len(), held);
        initial::yield_once().await;
    }

    // Counts and ids only: a log never carries what a message said.
    tracing::debug!(
        mailbox = mailbox.id.get(),
        listed = stored.len(),
        differing = differing.len(),
        corrected,
        "swept flags"
    );
    Ok(Swept {
        swept: true,
        corrected,
    })
}

#[cfg(test)]
mod tests {
    use postio_model::Flag;

    use super::*;

    fn flags(of: &[Flag]) -> FlagSet {
        of.iter().cloned().collect()
    }

    fn stored(rows: &[(i64, u32, &[Flag])]) -> Vec<(MessageId, Uid, FlagSet)> {
        rows.iter()
            .map(|(id, uid, of)| (MessageId::new(*id), Uid::new(*uid), flags(of)))
            .collect()
    }

    #[test]
    fn only_the_messages_whose_flags_differ_are_returned() {
        let held = stored(&[
            (10, 1, &[]),
            (11, 2, &[Flag::Seen]),
            (12, 3, &[Flag::Flagged]),
        ]);
        let server = vec![
            (Uid::new(1), flags(&[])),
            (Uid::new(2), flags(&[Flag::Seen, Flag::Answered])),
            (Uid::new(3), flags(&[])),
        ];

        let differing = disagreements(&held, server);

        assert_eq!(
            differing,
            vec![
                (MessageId::new(11), flags(&[Flag::Seen, Flag::Answered])),
                (MessageId::new(12), flags(&[])),
            ]
        );
    }

    #[test]
    fn an_arrival_and_a_deletion_are_not_a_flag_difference() {
        let held = stored(&[(10, 1, &[Flag::Seen])]);
        // UID 1 is gone from the server's list and UID 9 is not in the store.
        let server = vec![(Uid::new(9), flags(&[Flag::Seen]))];

        assert!(disagreements(&held, server).is_empty());
    }

    #[test]
    fn recent_is_not_a_difference() {
        let held = stored(&[(10, 1, &[Flag::Seen])]);
        let server = vec![(Uid::new(1), flags(&[Flag::Seen, Flag::Recent]))];

        assert!(disagreements(&held, server).is_empty());
    }
}
