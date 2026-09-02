//! How this adapter spells a [`RemoteId`] (#543, ADR 0018 Q2).
//!
//! The trait's addressing surface is the opaque `RemoteId`; IMAP's wire
//! wants a [`Uid`], and a `Uid` is meaningless without the
//! [`UidValidity`] it was observed under. So the adapter packs the pair
//! as `{uid_validity}:{uid}` — the exact spelling migration 0023
//! backfilled — and unpacks it at the wire. Nothing outside this crate
//! reads structure into a `RemoteId`, and [`wire`] answering `None` is
//! how an id from another generation (or another backend entirely)
//! surfaces: as "this mailbox needs a resync", never as a guessed uid.

use postio_model::{RemoteId, Uid, UidValidity};

/// The identity of a message observed as `uid` under `uid_validity`.
pub fn remote_id(uid_validity: UidValidity, uid: Uid) -> RemoteId {
    RemoteId::new(format!("{uid_validity}:{uid}"))
}

/// The wire pair a [`RemoteId`] of this adapter's spelling packs.
///
/// `None` for anything else — an id minted by another backend, or a
/// corrupted value. The caller treats that as a stale identity, not an
/// error to retry.
pub fn wire(remote_id: &RemoteId) -> Option<(UidValidity, Uid)> {
    let (validity, uid) = remote_id.as_str().split_once(':')?;
    Some((
        UidValidity::new(validity.parse().ok()?),
        Uid::new(uid.parse().ok()?),
    ))
}

/// Unpacks a batch of ids into the wire set, refusing any that are not of
/// this mailbox's live generation.
///
/// The refusal is [`crate::backend::BackendError::UidValidityChanged`] — the same answer a
/// renumber discovered at `SELECT` gives — because that is what a stale id
/// *is*: a name from a generation the server has abandoned. The caller's
/// recovery is identical: resync the mailbox, never retry the uid.
pub(crate) fn wire_set(
    mailbox: &str,
    live: UidValidity,
    ids: &[RemoteId],
) -> crate::backend::BackendResult<crate::backend::UidSet> {
    let mut set = crate::backend::UidSet::new();
    for id in ids {
        set.insert(wire_uid(mailbox, live, id)?);
    }
    Ok(set)
}

/// [`wire_set`], for the single id the body fetches address.
pub(crate) fn wire_uid(
    mailbox: &str,
    live: UidValidity,
    id: &RemoteId,
) -> crate::backend::BackendResult<Uid> {
    match wire(id) {
        Some((validity, uid)) if validity == live => Ok(uid),
        Some((validity, _)) => Err(crate::backend::BackendError::UidValidityChanged {
            mailbox: mailbox.to_owned(),
            known: validity,
            observed: live,
        }),
        // An id this adapter never minted — another backend's, or corrupt.
        // A caller bug, not a server condition; nothing to resync or retry.
        None => Err(crate::backend::BackendError::Protocol {
            reason: format!("remote id {id:?} is not this adapter's spelling"),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pair_round_trips_through_the_opaque_id() {
        let id = remote_id(UidValidity::new(4_242), Uid::new(12));
        assert_eq!(id.as_str(), "4242:12");
        assert_eq!(wire(&id), Some((UidValidity::new(4_242), Uid::new(12))));
    }

    #[test]
    fn an_id_from_another_backend_is_none_not_a_guess() {
        for foreign in ["gm-1234", "", ":", "7:", ":42", "a:b", "7:42:9"] {
            assert_eq!(wire(&RemoteId::new(foreign)), None, "{foreign:?}");
        }
    }
}
