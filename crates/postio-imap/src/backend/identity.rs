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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pair_round_trips_through_the_opaque_id() {
        let id = remote_id(UidValidity::new(4_242), Uid::new(12));
        assert_eq!(id.as_str(), "4242:12");
        assert_eq!(
            wire(&id),
            Some((UidValidity::new(4_242), Uid::new(12)))
        );
    }

    #[test]
    fn an_id_from_another_backend_is_none_not_a_guess() {
        for foreign in ["gm-1234", "", ":", "7:", ":42", "a:b", "7:42:9"] {
            assert_eq!(wire(&RemoteId::new(foreign)), None, "{foreign:?}");
        }
    }
}
