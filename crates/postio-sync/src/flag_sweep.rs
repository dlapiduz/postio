//! Compares the server's whole flag listing with the store's and writes only the messages that
//! disagree, keeping unsent local changes.
//! Only corrects flags: arrivals and deletions belong to `crate::resync`.

use std::collections::BTreeMap;

use postio_account::backend::MailBackend;
use postio_account::cancel::CancelToken;
use postio_model::{FlagSet, Generation, Mailbox, MessageId, Uid};
use postio_storage::Checkout;
use postio_storage::repository::MessageRepository;

use crate::drain::SyncError;
use crate::initial;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Swept {
    /// `false` when the backend cannot list flags alone, so the pass is not counted as having
    /// swept.
    pub swept: bool,
    pub corrected: usize,
}

/// Pure. `\Recent` is not compared: a server reporting it on one pass and not the next is no
/// change.
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

/// The caller must not record the server's `HIGHESTMODSEQ` until this returns: advancing past
/// unreconciled flags makes a missed change permanent.
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
