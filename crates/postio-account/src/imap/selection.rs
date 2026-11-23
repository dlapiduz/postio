//! A pooled session caches its selected mailbox so a backfill's chunks share one `SELECT`;
//! `CONDSTORE` stays active for the life of a selection (RFC 7162 §3.1.4).
//! The cached UID generation must not be trusted for long: an unnoticed renumber would land flags,
//! bodies and deletes on the wrong messages. So a selection expires
//! (`PoolConfig::selection_max_age`), a contradicting `SELECT` fails with `UidValidityChanged`, and
//! `Generations` is shared by the pool with an epoch that steps on each change.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use io_imap::client::ImapClientAsync;
use io_imap::rfc3501::examine::ImapMailboxExamineOptions;
use io_imap::rfc3501::select::{ImapMailboxSelectData, ImapMailboxSelectOptions};
use io_imap::types::command::SelectParameter;
use io_imap::types::flag::FlagPerm;
use io_imap::types::status::{StatusDataItem, StatusDataItemName};
use postio_model::{Flag, FlagSet, ModSeq, Uid, UidValidity};
use tokio::time::Instant;

use crate::backend::{BackendError, Capability, MailboxStatus, SelectMode};

use super::mailboxes::mailbox_argument;
use super::{ConnectionPool, ImapSession, Priority};
use crate::backend::BackendResult;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SelectedMailbox {
    path: String,
    condstore: bool,
    /// `EXAMINE`: a read-only selection can serve no write, so it is never a cache hit for anything
    /// else.
    read_only: bool,
    uid_validity: UidValidity,
    epoch: u64,
    confirmed_at: Instant,
    /// Goes stale only in the safe direction: a stale count is lower than the truth, so a check
    /// against it under-fires.
    exists: u32,
}

/// Shared, not per-connection: a renumber found by one connection must stop every other acting on
/// what it cached, and a fresh connection can still contradict what the pool believed.
#[derive(Debug, Default)]
pub(super) struct Generations {
    seen: Mutex<BTreeMap<String, Generation>>,
}

#[derive(Clone, Copy, Debug)]
struct Generation {
    uid_validity: UidValidity,
    epoch: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    Unchanged,
    Changed { known: UidValidity },
}

impl Generations {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Does not record: a disputed generation must be checked before it is written down
    /// (`select_now`).
    fn known(&self, path: &str) -> Option<UidValidity> {
        self.lock()
            .get(path)
            .map(|generation| generation.uid_validity)
    }

    fn epoch(&self, path: &str) -> Option<u64> {
        self.lock().get(path).map(|generation| generation.epoch)
    }

    /// `promised` is what this session last reported to a caller; it outranks the shared log,
    /// because that is the promise that would be broken.
    fn observe(
        &self,
        path: &str,
        promised: Option<UidValidity>,
        observed: UidValidity,
    ) -> (Verdict, u64) {
        let mut seen = self.lock();
        let known = promised.or_else(|| seen.get(path).map(|generation| generation.uid_validity));

        let generation = seen.entry(path.to_owned()).or_insert(Generation {
            uid_validity: observed,
            epoch: 0,
        });
        if generation.uid_validity != observed {
            generation.uid_validity = observed;
            // Steps once per change, which is what expires every *other*
            // session's cached selection for this mailbox.
            generation.epoch += 1;
        }
        let epoch = generation.epoch;

        match known {
            Some(known) if known != observed => (Verdict::Changed { known }, epoch),
            _ => (Verdict::Unchanged, epoch),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BTreeMap<String, Generation>> {
        self.seen
            .lock()
            .expect("the UID generation log is poisoned")
    }
}

impl ImapSession {
    pub(super) fn set_selection_policy(
        &mut self,
        generations: Arc<Generations>,
        max_age: Duration,
    ) {
        self.generations = generations;
        self.selection_max_age = max_age;
    }

    /// `want_condstore` needs `SELECT (CONDSTORE)` (RFC 7162 §3.3.1); `Unsupported` without the
    /// capability.
    /// A renumber since this session last reported fails with `UidValidityChanged`; the new
    /// generation is adopted first, so a retry succeeds.
    /// `selected_exists` is the `EXISTS` count for `path` if it is the selected mailbox.
    pub(super) fn selected_exists(&self, path: &str) -> Option<u32> {
        self.selected
            .as_ref()
            .filter(|selected| selected.path == path)
            .map(|selected| selected.exists)
    }

    pub(crate) async fn ensure_selected(
        &mut self,
        path: &str,
        want_condstore: bool,
    ) -> BackendResult<UidValidity> {
        if let Some(selected) = &self.selected
            && selected.path == path
            && !selected.read_only
            && (selected.condstore || !want_condstore)
            && Some(selected.epoch) == self.generations.epoch(path)
            && selected.confirmed_at.elapsed() < self.selection_max_age
        {
            return Ok(selected.uid_validity);
        }

        let data = self
            .select_now(path, SelectMode::ReadWrite, want_condstore)
            .await?;
        Ok(data.uid_validity)
    }

    async fn issue_select(
        &mut self,
        path: &str,
        mode: SelectMode,
        want_condstore: bool,
    ) -> BackendResult<(ImapMailboxSelectData, u64)> {
        let mailbox = mailbox_argument(path)?;
        let parameters = if want_condstore {
            vec![SelectParameter::CondStore]
        } else {
            Vec::new()
        };

        let output = if mode == SelectMode::ReadOnly {
            let options = ImapMailboxExamineOptions { parameters };
            let output = self.examine(mailbox, options).await;
            output.map_err(|error| self.command_error("EXAMINE", error))?
        } else {
            let options = ImapMailboxSelectOptions { parameters };
            let output = self.select(mailbox, options).await;
            output.map_err(|error| self.command_error("SELECT", error))?
        };
        let skipped = super::undecoded(&output);
        Ok((output.data, skipped))
    }

    /// The one place a `UIDVALIDITY` reaches the rest of the crate, so the check cannot be walked
    /// around.
    pub(super) async fn select_now(
        &mut self,
        path: &str,
        mode: SelectMode,
        want_condstore: bool,
    ) -> BackendResult<SelectedState> {
        if want_condstore {
            self.capabilities().require(Capability::CondStore)?;
        }

        let read_only = mode == SelectMode::ReadOnly;

        let (mut data, mut skipped) = self.issue_select(path, mode, want_condstore).await?;
        // An omitted `UIDVALIDITY` gets the same second chance as a disputed one: a server
        // contradicting itself under load also sent `SELECT`s without it (#1538).
        let mut uid_validity = match uid_validity_of(&data, path) {
            Ok(uid_validity) => uid_validity,
            Err(refusal) => {
                let (retried, retried_skips) =
                    self.issue_select(path, mode, want_condstore).await?;
                data = retried;
                skipped += retried_skips;
                match uid_validity_of(&data, path) {
                    Ok(uid_validity) => uid_validity,
                    // Twice missing: a server that never sends `UIDVALIDITY` is permanently
                    // unusable for UID work, but a line `io-imap` dropped is transient: the pool
                    // discards the connection and a fresh one retries. The response's `undecoded`
                    // count tells them apart.
                    Err(_) if skipped > 0 => {
                        return Err(BackendError::Disconnected {
                            context: format!("SELECT {path}"),
                            reason: format!(
                                "{path} SELECT carried no UIDVALIDITY and \
                                 {skipped} untagged response(s) were dropped \
                                 undecoded, so the line was lost rather than \
                                 never sent"
                            ),
                        });
                    }
                    Err(_) => return Err(refusal),
                }
            }
        };

        let promised = self
            .selected
            .as_ref()
            .filter(|selected| selected.path == path)
            .map(|selected| selected.uid_validity);

        // A renumber must be said twice before it is believed (#1538): `UidValidityChanged` empties
        // and refetches the mailbox, and a live account produced a burst of them while other
        // `SELECT`s came back with no `UIDVALIDITY`. One extra round trip, only on the path about
        // to throw a mailbox away.
        let known = promised.or_else(|| self.generations.known(path));
        if known.is_some_and(|known| known != uid_validity) {
            let disputed = uid_validity;
            let (rechecked, _) = self.issue_select(path, mode, want_condstore).await?;
            data = rechecked;
            uid_validity = uid_validity_of(&data, path)?;
            if uid_validity != disputed {
                tracing::warn!(
                    first = disputed.get(),
                    second = uid_validity.get(),
                    "the server gave two different UIDVALIDITYs in a row; \
                     believing the second and not resynchronising"
                );
            }
        }

        let (verdict, epoch) = self.generations.observe(path, promised, uid_validity);

        self.selected = Some(SelectedMailbox {
            path: path.to_owned(),
            condstore: want_condstore,
            read_only,
            uid_validity,
            epoch,
            confirmed_at: Instant::now(),
            exists: data.exists.unwrap_or_default(),
        });

        if let Verdict::Changed { known } = verdict {
            return Err(BackendError::UidValidityChanged {
                mailbox: path.to_owned(),
                known,
                observed: uid_validity,
            });
        }

        Ok(SelectedState {
            uid_validity,
            read_only,
            exists: data.exists.unwrap_or_default(),
            uid_next: data.uid_next.map(|value| Uid::new(value.get())),
            highest_mod_seq: data.highest_mod_seq.map(ModSeq::new),
            permanent_flags: data
                .permanent_flags
                .iter()
                .flatten()
                .filter_map(|flag| match flag {
                    FlagPerm::Flag(flag) => Some(Flag::parse(flag.to_string())),
                    FlagPerm::Asterisk => None,
                })
                .collect(),
            can_create_keywords: data
                .permanent_flags
                .iter()
                .flatten()
                .any(|flag| matches!(flag, FlagPerm::Asterisk)),
        })
    }
}

fn uid_validity_of(data: &ImapMailboxSelectData, path: &str) -> BackendResult<UidValidity> {
    data.uid_validity
        .map(|value| UidValidity::new(value.get()))
        .ok_or_else(|| BackendError::Protocol {
            reason: format!("{path} SELECT carried no UIDVALIDITY"),
        })
}

#[derive(Clone, Debug)]
pub(super) struct SelectedState {
    pub(super) uid_validity: UidValidity,
    pub(super) read_only: bool,
    pub(super) exists: u32,
    pub(super) uid_next: Option<Uid>,
    pub(super) highest_mod_seq: Option<ModSeq>,
    pub(super) permanent_flags: FlagSet,
    pub(super) can_create_keywords: bool,
}

/// `UNSEEN` is not carried across: `SELECT` reports the sequence number of the first unseen message
/// (RFC 3501 §7.1), not a count; `status` reports the real count.
#[tracing::instrument(skip_all, fields(mailbox = %path, mode = ?mode))]
pub async fn select(
    pool: &ConnectionPool,
    path: &str,
    mode: SelectMode,
    priority: Priority,
) -> BackendResult<MailboxStatus> {
    let path = path.to_owned();

    pool.execute(priority, async |session| {
        let condstore = session.capabilities().contains(Capability::CondStore);
        let state = session.select_now(&path, mode, condstore).await?;

        Ok(MailboxStatus {
            path: path.clone(),
            generation: postio_model::Generation::new(state.uid_validity.get()),
            uid_next: state.uid_next.unwrap_or_else(|| Uid::new(1)),
            exists: state.exists,
            unseen: None,
            highest_mod_seq: state.highest_mod_seq,
            permanent_flags: state.permanent_flags,
            can_create_keywords: state.can_create_keywords,
            read_only: state.read_only,
        })
    })
    .await
    .inspect(|status| {
        tracing::debug!(
            exists = status.exists,
            generation = status.generation.get(),
            uid_next = status.uid_next.get(),
            read_only = status.read_only,
            "selected"
        );
    })
    .inspect_err(|error| tracing::warn!(%error, "cannot select that folder"))
}

/// Outside the generation guard in `select_now`: `STATUS` hands back no UID to act on, so reporting
/// the current `UIDVALIDITY` serves a caller comparing it to its records better than an error.
pub async fn status(
    pool: &ConnectionPool,
    path: &str,
    priority: Priority,
) -> BackendResult<MailboxStatus> {
    let path = path.to_owned();

    pool.execute(priority, async |session| {
        let mut wanted = vec![
            StatusDataItemName::Messages,
            StatusDataItemName::UidNext,
            StatusDataItemName::UidValidity,
            StatusDataItemName::Unseen,
        ];
        if session.capabilities().contains(Capability::CondStore) {
            wanted.push(StatusDataItemName::HighestModSeq);
        }

        let mailbox = mailbox_argument(&path)?;
        let items = session.status(mailbox, Cow::Owned(wanted)).await;
        let items = items
            .map_err(|error| session.command_error("STATUS", error))?
            .data;

        let mut status = MailboxStatus {
            path: path.clone(),
            generation: postio_model::Generation::new(1),
            uid_next: Uid::new(1),
            exists: 0,
            unseen: None,
            highest_mod_seq: None,
            // STATUS says nothing about what a mailbox will accept.
            permanent_flags: FlagSet::new(),
            can_create_keywords: false,
            read_only: true,
        };

        for item in items {
            match item {
                StatusDataItem::Messages(count) => status.exists = count,
                StatusDataItem::UidNext(next) => status.uid_next = Uid::new(next.get()),
                StatusDataItem::UidValidity(value) => {
                    status.generation = postio_model::Generation::new(value.get());
                }
                StatusDataItem::Unseen(count) => status.unseen = Some(count),
                StatusDataItem::HighestModSeq(value) => {
                    status.highest_mod_seq = Some(ModSeq::new(value));
                }
                _ => {}
            }
        }

        Ok(status)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn generation(value: u32) -> UidValidity {
        UidValidity::new(value)
    }

    #[test]
    fn the_first_sight_of_a_mailbox_contradicts_nothing() {
        let generations = Generations::new();

        let (verdict, epoch) = generations.observe("INBOX", None, generation(1));

        assert_eq!(verdict, Verdict::Unchanged);
        assert_eq!(epoch, 0);
    }

    #[test]
    fn seeing_the_same_generation_again_is_not_a_change() {
        let generations = Generations::new();
        generations.observe("INBOX", None, generation(1));

        let (verdict, epoch) = generations.observe("INBOX", Some(generation(1)), generation(1));

        assert_eq!(verdict, Verdict::Unchanged);
        assert_eq!(epoch, 0);
    }

    #[test]
    fn a_renumber_is_reported_and_steps_the_epoch() {
        let generations = Generations::new();
        generations.observe("INBOX", None, generation(1));

        let (verdict, epoch) = generations.observe("INBOX", Some(generation(1)), generation(2));

        assert_eq!(
            verdict,
            Verdict::Changed {
                known: generation(1)
            }
        );
        assert_eq!(epoch, 1, "every cached selection for INBOX is now stale");
    }

    #[test]
    fn a_second_session_that_believed_the_old_generation_is_told_as_well() {
        // The first session already updated the log, so the log alone would say nothing changed.
        let generations = Generations::new();
        generations.observe("INBOX", None, generation(1));
        generations.observe("INBOX", Some(generation(1)), generation(2));

        let (verdict, epoch) = generations.observe("INBOX", Some(generation(1)), generation(2));

        assert_eq!(
            verdict,
            Verdict::Changed {
                known: generation(1)
            }
        );
        assert_eq!(epoch, 1, "the same change, not a second one");
    }

    #[test]
    fn a_fresh_session_is_still_told_what_the_pool_knew() {
        let generations = Generations::new();
        generations.observe("INBOX", None, generation(1));

        let (verdict, _) = generations.observe("INBOX", None, generation(2));

        assert_eq!(
            verdict,
            Verdict::Changed {
                known: generation(1)
            }
        );
    }

    #[test]
    fn mailboxes_are_tracked_apart() {
        let generations = Generations::new();
        generations.observe("INBOX", None, generation(1));

        let (verdict, epoch) = generations.observe("Archive", None, generation(7));

        assert_eq!(verdict, Verdict::Unchanged);
        assert_eq!(epoch, 0);
        assert_eq!(generations.epoch("INBOX"), Some(0));
    }
}
