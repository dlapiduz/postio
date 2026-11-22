//! A `RemoteId` is `{uid_validity}:{uid}` (migration 0023); `wire` returning `None` means a stale
//! or foreign id (#543, ADR 0018 Q2).

use postio_model::{RemoteId, Uid, UidValidity};

pub fn remote_id(uid_validity: UidValidity, uid: Uid) -> RemoteId {
    RemoteId::new(format!("{uid_validity}:{uid}"))
}

/// `None` for an id minted elsewhere or corrupt: the caller treats it as stale, not a retry.
pub fn wire(remote_id: &RemoteId) -> Option<(UidValidity, Uid)> {
    let (validity, uid) = remote_id.as_str().split_once(':')?;
    Some((
        UidValidity::new(validity.parse().ok()?),
        Uid::new(uid.parse().ok()?),
    ))
}

/// Refuses ids of another generation with `UidValidityChanged`, as a renumber at `SELECT` does:
/// resync, never retry the uid.
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
        // Not this adapter's id: a caller bug, nothing to resync or retry.
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
