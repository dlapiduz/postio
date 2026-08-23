//! Mailbox selection, cached per connection — and the UID generation that
//! cache is only ever allowed to hold briefly.
//!
//! # Why there is a cache
//!
//! A pooled connection is reused across many small fetches — a
//! ten-thousand-message backfill calls into the same session chunk after
//! chunk, per `UidSet::chunks` — and reselecting the same mailbox before
//! every one of them would double the round trips for no reason. So the
//! session remembers what it last selected, and in what mode, and only
//! issues `SELECT` again when the mailbox changes or a caller needs
//! `CONDSTORE` and the cached selection does not have it.
//!
//! `CONDSTORE`, once selected on a mailbox, stays active for the life of that
//! selection (RFC 7162 §3.1.4), which is what makes the "reuse if it already
//! has condstore" half of the cache check correct.
//!
//! # Why the cache cannot be trusted indefinitely
//!
//! `UIDVALIDITY` is the generation number of a mailbox's UID space, and a
//! server that restores from backup renumbers it. Every UID Postio holds then
//! means a different message, or none. Caching the generation alongside the
//! selection is what makes that renumber *invisible*: the connection is
//! parked, handed out again, and keeps answering with the generation it
//! learned before the world changed. UIDs from the new generation are then
//! read as though they were the old ones — flags land on the wrong messages,
//! bodies attach to the wrong rows, a delete hits mail the user never chose —
//! and nothing errors, because every layer above believes the generation it
//! was told.
//!
//! Three things together make that unreachable, at a cost of nothing in the
//! backfill case:
//!
//! 1. **A cached selection expires.** Past
//!    [`PoolConfig::selection_max_age`](super::PoolConfig::selection_max_age)
//!    the next use re-`SELECT`s. Chunks of one backfill follow each other in
//!    milliseconds and so still share a single `SELECT`; anything slower pays
//!    one extra round trip per mailbox per interval.
//! 2. **A contradiction is reported, never absorbed.** When a `SELECT`
//!    returns a generation different from the one this session last told a
//!    caller about, the operation fails with
//!    [`BackendError::UidValidityChanged`] — which
//!    [`requires_full_resync`](BackendError::requires_full_resync) — rather
//!    than quietly returning data from the new generation.
//! 3. **One connection's discovery invalidates every other's cache.**
//!    [`Generations`] is shared by every session in a pool and carries an
//!    epoch that steps on each change, so a cached selection made before the
//!    discovery stops being a cache hit immediately rather than at the end of
//!    its interval.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use io_imap::client::ImapClientAsync;
use io_imap::rfc3501::select::ImapMailboxSelectOptions;
use io_imap::types::command::SelectParameter;
use postio_model::UidValidity;
use tokio::time::Instant;

use crate::backend::{BackendError, Capability};

use super::mailboxes::mailbox_argument;
use super::{ImapSession, map_client_error};
use crate::backend::BackendResult;

/// What is currently selected on a session, and how.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SelectedMailbox {
    path: String,
    condstore: bool,
    uid_validity: UidValidity,
    /// The generation epoch this selection was confirmed against. A newer one
    /// means somebody else has since seen this mailbox renumbered.
    epoch: u64,
    /// When the server last confirmed it.
    confirmed_at: Instant,
}

/// The UID generation every session in a pool has observed, per mailbox.
///
/// Shared rather than per-connection on purpose: a renumber discovered by one
/// connection has to stop every other connection from acting on what it
/// cached before, and a fresh connection opened after the event still has to
/// be able to contradict what the pool believed.
#[derive(Debug, Default)]
pub(super) struct Generations {
    seen: Mutex<BTreeMap<String, Generation>>,
}

#[derive(Clone, Copy, Debug)]
struct Generation {
    uid_validity: UidValidity,
    epoch: u64,
}

/// What a fresh `SELECT` means for the caller that asked for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// The generation is the one everybody already believed.
    Unchanged,
    /// It is not, and whoever was told `known` has to be told so.
    Changed { known: UidValidity },
}

impl Generations {
    /// An empty log.
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The current epoch for `path`, if anything has ever been observed.
    fn epoch(&self, path: &str) -> Option<u64> {
        self.lock().get(path).map(|generation| generation.epoch)
    }

    /// Records an observation and says what it means.
    ///
    /// `promised` is what the asking session last reported to a caller for
    /// this mailbox. It takes precedence over the shared log because it is
    /// the promise that would be broken: a second session that also believed
    /// the old generation must be told even though the first one has already
    /// updated the log.
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
    /// Puts this session under a pool's selection policy.
    ///
    /// Called when the pool opens a connection, so every session in one pool
    /// shares one view of each mailbox's generation. A session opened outside
    /// a pool keeps its own.
    pub(super) fn set_selection_policy(
        &mut self,
        generations: Arc<Generations>,
        max_age: Duration,
    ) {
        self.generations = generations;
        self.selection_max_age = max_age;
    }

    /// Selects `path` unless it is already selected in a mode that covers
    /// what the caller needs, and returns its `UIDVALIDITY`.
    ///
    /// `want_condstore` asks for `SELECT (CONDSTORE)`, which RFC 7162 §3.3.1
    /// requires before a `FETCH … (CHANGEDSINCE n)` on this mailbox. Asking
    /// for it against a server that never advertised `CONDSTORE` is
    /// [`BackendError::Unsupported`], not a silent fallback.
    ///
    /// # Errors
    ///
    /// [`BackendError::UidValidityChanged`] when the mailbox has been
    /// renumbered since this session last reported a generation for it. The
    /// new generation is adopted before the error is returned, so the caller
    /// that rebuilds and retries succeeds rather than looping — see the
    /// [module docs](self).
    pub(crate) async fn ensure_selected(
        &mut self,
        path: &str,
        want_condstore: bool,
    ) -> BackendResult<UidValidity> {
        if let Some(selected) = &self.selected
            && selected.path == path
            && (selected.condstore || !want_condstore)
            && Some(selected.epoch) == self.generations.epoch(path)
            && selected.confirmed_at.elapsed() < self.selection_max_age
        {
            return Ok(selected.uid_validity);
        }

        if want_condstore {
            self.capabilities().require(Capability::CondStore)?;
        }

        let mailbox = mailbox_argument(path)?;
        let parameters = if want_condstore {
            vec![SelectParameter::CondStore]
        } else {
            Vec::new()
        };

        let data = self
            .select(mailbox, ImapMailboxSelectOptions { parameters })
            .await
            .map_err(|error| map_client_error("SELECT", self.account(), error))?;

        let uid_validity = data
            .uid_validity
            .map(|value| UidValidity::new(value.get()))
            .ok_or_else(|| BackendError::Protocol {
                reason: format!("{path} SELECT carried no UIDVALIDITY"),
            })?;

        let promised = self
            .selected
            .as_ref()
            .filter(|selected| selected.path == path)
            .map(|selected| selected.uid_validity);
        let (verdict, epoch) = self.generations.observe(path, promised, uid_validity);

        self.selected = Some(SelectedMailbox {
            path: path.to_owned(),
            condstore: want_condstore,
            uid_validity,
            epoch,
            confirmed_at: Instant::now(),
        });

        match verdict {
            Verdict::Unchanged => Ok(uid_validity),
            Verdict::Changed { known } => Err(BackendError::UidValidityChanged {
                mailbox: path.to_owned(),
                known,
                observed: uid_validity,
            }),
        }
    }
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
        // The first session has already updated the log, so the log alone
        // would say "nothing changed" — and the second session's caller,
        // which was handed the old generation, would never hear about it.
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
        // Opened after the renumber, so it promised nothing itself. The pool
        // is what remembers, and the caller still has to rebuild.
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
