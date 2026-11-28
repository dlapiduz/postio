//! Each command depends on a capability, chosen once in `Dispatch`. `EXPUNGE` always returns an
//! empty list: the wire carries sequence numbers and this layer keeps no sequence-to-UID map, since
//! a plausible wrong id is worse than none; callers resync.
//! A destination UID is reported only when the server sent `COPYUID`/`APPENDUID`.

use chrono::{SubsecRound, Utc};
use io_imap::client::ImapClientAsync;
use io_imap::rfc3501::append::ImapMessageAppendOptions;
use io_imap::rfc3501::copy::{ImapCopyUid, ImapMessageCopyOptions};
use io_imap::rfc3501::store::ImapMessageStoreOptions;
use io_imap::rfc6851::r#move::ImapMessageMoveOptions;
use io_imap::types::IntoStatic;
use io_imap::types::datetime::DateTime as WireDateTime;
use io_imap::types::fetch::MessageDataItem;
use io_imap::types::flag::{Flag as WireFlag, StoreType};
use io_imap::types::sequence::SequenceSet;
use postio_model::{Flag, FlagSet, ModSeq, RemoteId, Uid, UidValidity};

use crate::backend::{
    AppendMessage, BackendError, BackendResult, Capability, FlagChange, FlagUpdate, UidMapping,
    UidSet, identity,
};

use super::fetch::flag_from_wire;
use super::mailboxes::mailbox_argument;
use super::{ConnectionPool, Dispatch, ExpungeStrategy, ImapSession, MoveStrategy, Priority};

/// Last-writer-wins: conditional `STORE (UNCHANGEDSINCE n)` is not reachable through `io-imap` (ADR
/// 0001 Q2 gap 1), so a flag lost to a race returns on the next resync.
pub async fn store_flags(
    pool: &ConnectionPool,
    mailbox: &str,
    ids: &[RemoteId],
    change: &FlagChange,
    priority: Priority,
) -> BackendResult<Vec<FlagUpdate>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let mailbox = mailbox.to_owned();
    let change = change.clone();

    pool.execute(priority, async |session| {
        let live = session.ensure_selected(&mailbox, false).await?;
        let uids = identity::wire_set(&mailbox, live, ids)?;
        store(session, live, &uids, &change).await
    })
    .await
}

/// `CopyThenDelete` is three commands and not atomic: a failure between copy and store leaves the
/// message in both mailboxes; the caller's resync settles it.
pub async fn move_messages(
    pool: &ConnectionPool,
    from: &str,
    ids: &[RemoteId],
    to: &str,
    priority: Priority,
) -> BackendResult<Vec<UidMapping>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let from = from.to_owned();
    let to = to.to_owned();

    pool.execute(priority, async |session| {
        let strategy = Dispatch::new(session.capabilities().clone()).move_strategy();
        let live = session.ensure_selected(&from, false).await?;
        let uids = &identity::wire_set(&from, live, ids)?;

        match strategy {
            MoveStrategy::Move => {
                let set = sequence_set_for(uids)?;
                let destination = mailbox_argument(&to)?;
                let options = ImapMessageMoveOptions { uid: true };
                let moved = session.r#move(set, destination, options).await;
                let moved = moved.map_err(|error| session.command_error("MOVE", error))?;
                Ok(mapping_from(moved))
            }
            MoveStrategy::CopyThenDelete { uid_expunge } => {
                let mapping = copy(session, uids, &to).await?;
                store(
                    session,
                    live,
                    uids,
                    &FlagChange::Add(FlagSet::from_iter([Flag::Deleted])),
                )
                .await?;
                if uid_expunge {
                    let set = sequence_set_for(uids)?;
                    let expunged = session.uid_expunge(set).await;
                    expunged.map_err(|error| session.command_error("UID EXPUNGE", error))?;
                }
                Ok(mapping)
            }
        }
    })
    .await
}

pub async fn copy_messages(
    pool: &ConnectionPool,
    from: &str,
    ids: &[RemoteId],
    to: &str,
    priority: Priority,
) -> BackendResult<Vec<UidMapping>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    let from = from.to_owned();
    let to = to.to_owned();

    pool.execute(priority, async |session| {
        let live = session.ensure_selected(&from, false).await?;
        let uids = identity::wire_set(&from, live, ids)?;
        copy(session, &uids, &to).await
    })
    .await
}

/// With `uids`, on a server without UIDPLUS nothing is sent (`Defer`): a bare `EXPUNGE` would
/// destroy another client's marked mail. Always returns an empty list.
pub async fn expunge(
    pool: &ConnectionPool,
    mailbox: &str,
    ids: Option<&[RemoteId]>,
    priority: Priority,
) -> BackendResult<Vec<RemoteId>> {
    let mailbox = mailbox.to_owned();
    let targeted = ids.map(<[RemoteId]>::to_vec);

    pool.execute(priority, async |session| {
        let strategy =
            Dispatch::new(session.capabilities().clone()).expunge_strategy(targeted.is_some());
        if strategy == ExpungeStrategy::Defer {
            return Ok(Vec::new());
        }

        let live = session.ensure_selected(&mailbox, false).await?;
        let targeted = targeted
            .as_deref()
            .map(|ids| identity::wire_set(&mailbox, live, ids))
            .transpose()?;

        match strategy {
            ExpungeStrategy::UidExpunge => {
                let set = sequence_set_for(targeted.as_ref().expect("targeted"))?;
                let expunged = session.uid_expunge(set).await;
                expunged.map_err(|error| session.command_error("UID EXPUNGE", error))?;
            }
            ExpungeStrategy::Expunge => {
                let expunged = session.expunge().await;
                expunged.map_err(|error| session.command_error("EXPUNGE", error))?;
            }
            ExpungeStrategy::Defer => unreachable!("returned above"),
        }

        Ok(Vec::new())
    })
    .await
}

pub async fn append(
    pool: &ConnectionPool,
    mailbox: &str,
    message: &AppendMessage,
    priority: Priority,
) -> BackendResult<Option<UidMapping>> {
    let mailbox = mailbox.to_owned();
    let raw = message.raw.clone();
    let flags = wire_flags(&message.flags)?;
    let date = message.internal_date.map(wire_date).transpose()?;

    pool.execute(priority, async |session| {
        let target = mailbox_argument(&mailbox)?;
        let options = ImapMessageAppendOptions {
            flags: flags.clone(),
            date: date.clone(),
            // A non-synchronising literal needs LITERAL+ and gives up the
            // server's chance to reject the message before we send all of it.
            non_sync: false,
        };

        let appended = session.append(target, &raw, options).await;
        let (_, uid) = appended.map_err(|error| session.command_error("APPEND", error))?;

        Ok(uid.map(|(uid_validity, uid)| UidMapping {
            // An appended message has no source; it is its own origin.
            source: Uid::new(uid),
            destination: Uid::new(uid),
            uid_validity: UidValidity::new(uid_validity),
            destination_remote_id: identity::remote_id(
                UidValidity::new(uid_validity),
                Uid::new(uid),
            ),
        }))
    })
    .await
}

/// The cross-account saga's confirmation fallback (#188). The newest match wins when several: any
/// copy proves arrival.
pub async fn find_by_message_id(
    pool: &ConnectionPool,
    mailbox: &str,
    message_id: &str,
    priority: Priority,
) -> BackendResult<Option<postio_model::RemoteId>> {
    use io_imap::rfc3501::search::ImapMessageSearchOptions;
    use io_imap::types::core::{AString, Vec1};
    use io_imap::types::search::SearchKey;

    let mailbox = mailbox.to_owned();
    let message_id = message_id.to_owned();

    pool.execute(priority, async |session| {
        let live = session.ensure_selected(&mailbox, false).await?;
        let header =
            AString::try_from("Message-ID".to_owned()).map_err(|error| BackendError::Protocol {
                reason: format!("Message-ID is not an IMAP astring: {error}"),
            })?;
        let value =
            AString::try_from(message_id.clone()).map_err(|error| BackendError::Protocol {
                reason: format!("`{message_id}` is not an IMAP astring: {error}"),
            })?;
        let criteria = Vec1::from(SearchKey::Header(header, value));
        let found = session
            .search(criteria, ImapMessageSearchOptions { uid: true })
            .await
            .map_err(|error| session.command_error("SEARCH", error))?;
        Ok(found
            .data
            .into_iter()
            .max()
            .map(|uid| identity::remote_id(live, postio_model::Uid::new(uid.get()))))
    })
    .await
}

async fn store(
    session: &mut ImapSession,
    live: UidValidity,
    uids: &UidSet,
    change: &FlagChange,
) -> BackendResult<Vec<FlagUpdate>> {
    let set = sequence_set_for(uids)?;
    let (kind, flags) = match change {
        FlagChange::Add(flags) => (StoreType::Add, wire_flags(flags)?),
        FlagChange::Remove(flags) => (StoreType::Remove, wire_flags(flags)?),
        FlagChange::Replace(flags) => (StoreType::Replace, wire_flags(flags)?),
    };
    let options = ImapMessageStoreOptions { uid: true };

    let echoes = session.store(set, kind, flags, options).await;
    let echoes = echoes.map_err(|error| session.command_error("STORE", error))?;
    let condstore = session.capabilities().contains(Capability::CondStore);

    Ok(echoes
        .data
        .into_values()
        .filter_map(|items| update_from(live, items.into_iter().collect(), condstore))
        .collect())
}

async fn copy(
    session: &mut ImapSession,
    uids: &UidSet,
    to: &str,
) -> BackendResult<Vec<UidMapping>> {
    let set = sequence_set_for(uids)?;
    let destination = mailbox_argument(to)?;
    let options = ImapMessageCopyOptions { uid: true };

    let copied = session.copy(set, destination, options).await;
    let copied = copied.map_err(|error| session.command_error("COPY", error))?;
    Ok(mapping_from(copied))
}

/// The two sequence sets are positionally matched (RFC 4315 §3); io-imap gives no code when their
/// sizes differ. A UIDVALIDITY of `0` (Mailo sends it) yields nothing: an id built on it cannot be
/// found again, so the caller looks the copies up.
fn mapping_from(copied: Option<ImapCopyUid>) -> Vec<UidMapping> {
    let Some(copied) = copied.filter(|copied| copied.uid_validity != 0) else {
        return Vec::new();
    };
    let uid_validity = UidValidity::new(copied.uid_validity);

    copied
        .pairs()
        .map(|(source, destination)| UidMapping {
            source: Uid::new(source.get()),
            destination: Uid::new(destination.get()),
            uid_validity,
            destination_remote_id: identity::remote_id(uid_validity, Uid::new(destination.get())),
        })
        .collect()
}

/// An echo without a UID is dropped: it cannot name its message.
fn update_from(
    live: UidValidity,
    items: Vec<MessageDataItem<'static>>,
    condstore: bool,
) -> Option<FlagUpdate> {
    let mut uid = None;
    let mut flags = FlagSet::new();
    let mut mod_seq = None;

    for item in items {
        match item {
            MessageDataItem::Uid(value) => uid = Some(Uid::new(value.get())),
            MessageDataItem::Flags(list) => {
                flags = list.into_iter().map(flag_from_wire).collect();
            }
            MessageDataItem::ModSeq(value) => mod_seq = Some(ModSeq::new(value.get())),
            _ => {}
        }
    }

    Some(FlagUpdate {
        remote_id: identity::remote_id(live, uid?),
        flags,
        mod_seq: condstore.then_some(mod_seq).flatten(),
    })
}

fn sequence_set_for(uids: &UidSet) -> BackendResult<SequenceSet> {
    SequenceSet::try_from(uids.to_sequence_set().as_str()).map_err(|error| BackendError::Protocol {
        reason: format!("{uids} is not a sequence set IMAP can carry: {error}"),
    })
}

fn wire_flags(flags: &FlagSet) -> BackendResult<Vec<WireFlag<'static>>> {
    flags
        .iter()
        // `\Recent` is the server's alone to set (RFC 3501 §2.3.2).
        .filter(|flag| !flag.is_transient())
        .map(|flag| {
            let spelling = flag.as_str().to_owned();
            WireFlag::try_from(spelling.as_str())
                .map(|flag| flag.into_static())
                .map_err(|error| BackendError::Protocol {
                    reason: format!("{spelling:?} is not a flag IMAP can carry: {error}"),
                })
        })
        .collect()
}

/// IMAP carries whole seconds, so a sub-second timestamp is truncated rather than refused.
fn wire_date(at: chrono::DateTime<Utc>) -> BackendResult<WireDateTime> {
    let at = at.trunc_subsecs(0).fixed_offset();
    WireDateTime::try_from(at).map_err(|error| BackendError::Protocol {
        reason: format!("{at} is not a date IMAP can carry: {error}"),
    })
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use io_imap::types::flag::FlagFetch;

    use super::*;

    fn item(items: Vec<MessageDataItem<'static>>) -> Option<FlagUpdate> {
        update_from(UidValidity::new(1), items, true)
    }

    #[test]
    fn recent_is_never_put_on_the_wire() {
        let flags = FlagSet::from_iter([Flag::Seen, Flag::Recent]);

        let wire = wire_flags(&flags).expect("flags");

        let spelled: Vec<String> = wire.iter().map(ToString::to_string).collect();
        assert_eq!(spelled, ["\\Seen"]);
    }

    fn seq(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("nonzero")
    }

    #[test]
    fn an_echo_without_a_uid_names_nothing_and_is_dropped() {
        assert!(
            item(vec![MessageDataItem::Flags(vec![FlagFetch::Flag(
                WireFlag::Seen
            )])])
            .is_none()
        );
    }

    #[test]
    fn an_echo_carries_the_flags_and_the_sequence_it_was_stamped_with() {
        let update = item(vec![
            MessageDataItem::Uid(seq(7)),
            MessageDataItem::Flags(vec![FlagFetch::Flag(WireFlag::Seen)]),
            MessageDataItem::ModSeq(std::num::NonZeroU64::new(901).expect("nonzero")),
        ])
        .expect("a named update");

        assert_eq!(update.remote_id, RemoteId::new("1:7"));
        assert!(update.flags.is_seen());
        assert_eq!(update.mod_seq, Some(ModSeq::new(901)));
    }

    #[test]
    fn a_server_without_condstore_reports_no_modification_sequence() {
        let update = update_from(
            UidValidity::new(1),
            vec![
                MessageDataItem::Uid(seq(7)),
                MessageDataItem::Flags(vec![FlagFetch::Flag(WireFlag::Seen)]),
            ],
            false,
        )
        .expect("a named update");

        assert_eq!(update.mod_seq, None);
    }

    fn uids(first: u32, last: u32) -> std::ops::RangeInclusive<NonZeroU32> {
        NonZeroU32::new(first).unwrap()..=NonZeroU32::new(last).unwrap()
    }

    #[test]
    fn a_copyuid_pairs_each_source_with_its_destination() {
        let mapping = mapping_from(Some(ImapCopyUid {
            uid_validity: 4_242,
            source: vec![uids(1, 2), uids(7, 7)],
            destination: vec![uids(9, 11)],
        }));

        let pairs: Vec<(Uid, Uid)> = mapping
            .iter()
            .map(|mapped| (mapped.source, mapped.destination))
            .collect();
        assert_eq!(
            pairs,
            [
                (Uid::new(1), Uid::new(9)),
                (Uid::new(2), Uid::new(10)),
                (Uid::new(7), Uid::new(11)),
            ]
        );
        assert!(
            mapping
                .iter()
                .all(|mapped| mapped.uid_validity == UidValidity::new(4_242))
        );
    }

    #[test]
    fn a_copyuid_naming_generation_zero_is_no_mapping() {
        let mapping = mapping_from(Some(ImapCopyUid {
            uid_validity: 0,
            source: vec![uids(1, 1)],
            destination: vec![uids(9, 9)],
        }));

        assert!(mapping.is_empty());
    }

    #[test]
    fn no_copyuid_is_no_mapping_rather_than_a_guess() {
        assert!(mapping_from(None).is_empty());
    }

    #[test]
    fn a_keyword_survives_the_trip_to_the_wire() {
        let flags = FlagSet::from_iter([Flag::Seen, Flag::parse("$label1")]);
        let wire: Vec<String> = wire_flags(&flags)
            .expect("both are carryable")
            .iter()
            .map(ToString::to_string)
            .collect();

        assert!(wire.iter().any(|flag| flag == "\\Seen"));
        assert!(wire.iter().any(|flag| flag == "$label1"));
    }

    #[test]
    fn a_sub_second_internal_date_is_truncated_rather_than_refused() {
        let at = chrono::DateTime::parse_from_rfc3339("2026-08-23T12:00:00.750Z")
            .expect("a valid timestamp")
            .with_timezone(&Utc);

        let carried = wire_date(at).expect("a carryable date");

        assert_eq!(carried.as_ref().timestamp_subsec_nanos(), 0);
        assert_eq!(carried.as_ref().timestamp(), at.timestamp());
    }
}
