//! Watching one mailbox for change: `IDLE` where the server has it, `STATUS`
//! polling where it does not.
//!
//! # What comes back
//!
//! Deliberately raw events. `IDLE` says *that* a mailbox changed, not what it
//! now holds: `EXPUNGE` carries a sequence number rather than a UID, `FETCH`
//! carries whichever items the server felt like volunteering, and none of it
//! is a diff that can be applied. The answer to any of them is a resync pull.
//! Returning them unchewed is what keeps that decision with the sync engine.
//!
//! # Re-arming belongs to the caller
//!
//! A server is entitled to drop an `IDLE` that has run too long -- RFC 2177
//! §3 allows it after 29 minutes, and NAT middle-boxes are far less patient
//! -- so a watcher has to wind `IDLE` down with a clean `DONE` and arm it
//! again. How often is policy, and it has one home:
//! `postio_sync::WatchPolicy::idle_refresh`, which the watcher hands to
//! [`idle`] as `timeout`. This module waits for exactly that long and no
//! longer, in one `IDLE` ... `DONE` round, so there is no second interval
//! here for the two to disagree about.
//!
//! The same goes for a server without `IDLE`: the watcher polls it with
//! `STATUS` on `WatchPolicy::poll_interval` and never calls [`idle`].
//!
//! # Gated on `CAPABILITY`, never on an echo
//!
//! Whether `IDLE` is used at all comes from the capability list read *after*
//! authentication (ADR 0001, Q3), by way of [`WatchStrategy`]. It is never
//! inferred from an `* ENABLED` echo — at least one mainstream provider omits
//! that line entirely, and a client that reads its absence as "unsupported"
//! quietly falls back to polling forever.
//!
//! `watch::ImapMailboxWatch` is not used here, on purpose: it seeds an
//! in-memory shadow of the whole mailbox from a `FETCH 1:*` and holds it for
//! the life of the watch, which is exactly the "never load a whole mailbox
//! into memory" invariant this project is built on. ADR 0001, Q2 gap 4.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use io_imap::coroutine::{ImapCoroutine, ImapCoroutineState};
use io_imap::rfc2177::idle::{
    ImapIdle, ImapIdleError, ImapIdleEvent, ImapIdleOptions, ImapIdleYield,
};
use io_imap::types::response::Data;
use postio_model::{FlagSet, Uid};

use crate::backend::{BackendError, BackendResult, Capability, MailboxEvent};
use crate::cancel::CancelToken;

use super::fetch::flag_from_wire;
use super::{ConnectionPool, Dispatch, ImapSession, READ_BUFFER, WatchStrategy};

/// Waits for the server to say something about `mailbox`.
///
/// Returns as soon as anything arrives, when `timeout` elapses, or when
/// `cancel` fires — the last two with an empty vector, because "nothing
/// happened" is not a failure.
///
/// Takes the pool's watch connection, so a wait of minutes never occupies a
/// slot that interactive work needs.
pub async fn idle(
    pool: &ConnectionPool,
    mailbox: &str,
    timeout: Duration,
    cancel: &CancelToken,
) -> BackendResult<Vec<MailboxEvent>> {
    if cancel.is_cancelled() {
        return Ok(Vec::new());
    }

    let mailbox = mailbox.to_owned();

    pool.watch(async |session| {
        // From this session's own capability list rather than a pool-level
        // cache, so the very first watch of an account decides the same way
        // as every later one.
        match Dispatch::new(session.capabilities().clone()).watch_strategy() {
            WatchStrategy::Idle => {
                session.ensure_selected(&mailbox, false).await?;
                round_of_idle(session, timeout, cancel).await
            }
            // Polling is the watcher's job (`WatchPolicy::poll_interval`):
            // it asks `status` and never reaches here for such a server.
            WatchStrategy::Poll => Err(BackendError::Unsupported {
                capability: Capability::Idle,
            }),
        }
    })
    .await
}

/// One `IDLE` … `DONE` round, at most `window` long.
///
/// Always winds down with `DONE` rather than dropping the connection: the
/// session goes back to the pool usable, which is what makes re-arming cheap
/// enough to do often.
async fn round_of_idle(
    session: &mut ImapSession,
    window: Duration,
    cancel: &CancelToken,
) -> BackendResult<Vec<MailboxEvent>> {
    let shutdown = Arc::new(AtomicBool::new(false));
    // The coroutine's own timer is a backstop; this loop decides when the
    // round ends. It is set past the window, never at it: when the two
    // expire together the coroutine re-arms `IDLE` itself after the `DONE`
    // this loop asked for, and the round that was meant to end goes on.
    let mut coroutine = ImapIdle::new(
        Arc::clone(&shutdown),
        ImapIdleOptions {
            timeout: Some(window.saturating_add(Duration::from_secs(60))),
        },
    );

    let mut buffer = [0u8; READ_BUFFER];
    let mut resume: Option<&[u8]> = None;
    let mut events = Vec::new();
    let mut winding_down = false;

    let until = tokio::time::sleep(window);
    tokio::pin!(until);

    loop {
        match coroutine.resume(&mut session.fragmentizer, resume.take()) {
            ImapCoroutineState::Complete(Ok(())) => return Ok(events),
            ImapCoroutineState::Complete(Err(error)) => return Err(map_idle_error(error)),
            ImapCoroutineState::Yielded(ImapIdleYield::WantsWrite(bytes)) => {
                let timeout = session.command_timeout();
                session.stream.write_within(&bytes, timeout, "IDLE").await?;
            }
            ImapCoroutineState::Yielded(ImapIdleYield::Event(event)) => {
                events.extend(translate(event));
                // Something happened. Stop idling and hand it up: the answer
                // is a resync pull, not another wait.
                shutdown.store(true, Ordering::SeqCst);
                winding_down = true;
            }
            ImapCoroutineState::Yielded(ImapIdleYield::WantsRead) => {
                let read = if winding_down {
                    // Waiting for the tagged response to `DONE`, which is an
                    // ordinary command and bounded like one.
                    let timeout = session.command_timeout();
                    Some(
                        session
                            .stream
                            .read_within(&mut buffer, timeout, "IDLE DONE")
                            .await?,
                    )
                } else {
                    // Idling is silence by design, so the command deadline
                    // does not apply: this wait is bounded by the round and
                    // by the caller instead.
                    tokio::select! {
                        read = session.stream.read_within(&mut buffer, Duration::ZERO, "IDLE") => {
                            Some(read?)
                        }
                        _ = &mut until => {
                            shutdown.store(true, Ordering::SeqCst);
                            winding_down = true;
                            None
                        }
                        _ = cancel.cancelled() => {
                            shutdown.store(true, Ordering::SeqCst);
                            winding_down = true;
                            None
                        }
                    }
                };

                resume = read.map(|read| &buffer[..read]);
            }
        }
    }
}

/// Turns one batch of untagged responses into events.
///
/// Anything not in [`MailboxEvent`]'s vocabulary is dropped rather than
/// guessed at: `RECENT` is meaningless to a client that keeps its own state,
/// and a `FETCH` with no flags in it says nothing a resync will not say
/// better.
fn translate(event: ImapIdleEvent) -> Vec<MailboxEvent> {
    let mut events = Vec::new();

    for data in event.data {
        match data {
            Data::Exists(count) => events.push(MailboxEvent::Exists { count }),
            Data::Expunge(seq) => events.push(MailboxEvent::Expunged { seq: seq.get() }),
            Data::Vanished { known_uids, .. } => {
                let largest = std::num::NonZeroU32::new(u32::MAX).expect("u32::MAX is nonzero");
                events.push(MailboxEvent::Vanished {
                    uids: known_uids
                        .iter(largest)
                        .map(|uid| Uid::new(uid.get()))
                        .collect(),
                });
            }
            Data::Fetch { items, .. } => {
                let mut uid = None;
                let mut flags = None;
                for item in items {
                    match item {
                        io_imap::types::fetch::MessageDataItem::Uid(value) => {
                            uid = Some(Uid::new(value.get()));
                        }
                        io_imap::types::fetch::MessageDataItem::Flags(list) => {
                            flags = Some(list.into_iter().map(flag_from_wire).collect::<FlagSet>());
                        }
                        _ => {}
                    }
                }
                if let Some(flags) = flags {
                    events.push(MailboxEvent::FlagsChanged { uid, flags });
                }
            }
            _ => {}
        }
    }

    events
}

fn map_idle_error(error: ImapIdleError) -> BackendError {
    match error {
        ImapIdleError::No(reason) | ImapIdleError::Bad(reason) => BackendError::Rejected {
            command: "IDLE".to_owned(),
            reason,
        },
        // The server hung up on the watcher. Transient by nature: the watch is
        // re-established on a fresh connection rather than abandoned.
        ImapIdleError::Bye(reason) => BackendError::Disconnected {
            context: "IDLE".to_owned(),
            reason,
        },
        ImapIdleError::Eof | ImapIdleError::MissingTagged => BackendError::Disconnected {
            context: "IDLE".to_owned(),
            reason: error.to_string(),
        },
        other => BackendError::Protocol {
            reason: other.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU32;

    use io_imap::types::core::Vec1;
    use io_imap::types::fetch::MessageDataItem;
    use io_imap::types::flag::{Flag as WireFlag, FlagFetch};
    use io_imap::types::sequence::SequenceSet;
    use postio_model::Flag;

    use super::*;

    fn batch(data: Vec<Data<'static>>) -> Vec<MailboxEvent> {
        translate(ImapIdleEvent {
            untagged: Vec::new(),
            data,
        })
    }

    fn seq(value: u32) -> NonZeroU32 {
        NonZeroU32::new(value).expect("a nonzero sequence number")
    }

    #[test]
    fn an_exists_is_the_count_the_server_reported() {
        assert_eq!(
            batch(vec![Data::Exists(42)]),
            vec![MailboxEvent::Exists { count: 42 }]
        );
    }

    #[test]
    fn an_expunge_keeps_its_sequence_number() {
        // Not a UID, and not convertible into one without state the sync
        // engine owns. Reporting it as what it is stops anybody applying it
        // to the wrong message.
        assert_eq!(
            batch(vec![Data::Expunge(seq(3))]),
            vec![MailboxEvent::Expunged { seq: 3 }]
        );
    }

    #[test]
    fn a_vanished_set_is_expanded_into_uids() {
        let events = batch(vec![Data::Vanished {
            earlier: false,
            known_uids: SequenceSet::try_from("1:3,7").expect("a sequence set"),
        }]);

        assert_eq!(
            events,
            vec![MailboxEvent::Vanished {
                uids: [1, 2, 3, 7].into_iter().map(Uid::new).collect()
            }]
        );
    }

    #[test]
    fn a_fetch_with_flags_becomes_a_flag_change() {
        let events = batch(vec![Data::Fetch {
            seq: seq(1),
            items: Vec1::try_from(vec![
                MessageDataItem::Uid(seq(9)),
                MessageDataItem::Flags(vec![FlagFetch::Flag(WireFlag::Seen)]),
            ])
            .expect("a nonempty item list"),
        }]);

        assert_eq!(
            events,
            vec![MailboxEvent::FlagsChanged {
                uid: Some(Uid::new(9)),
                flags: FlagSet::from_iter([Flag::Seen]),
            }]
        );
    }

    #[test]
    fn a_fetch_without_flags_says_nothing_worth_reporting() {
        // A resync will say it better, and inventing an event here would put
        // a change into the sync engine that the server never described.
        let events = batch(vec![Data::Fetch {
            seq: seq(1),
            items: Vec1::from(MessageDataItem::Uid(seq(9))),
        }]);

        assert!(events.is_empty());
    }

    #[test]
    fn responses_outside_the_vocabulary_are_dropped_rather_than_guessed_at() {
        assert!(batch(vec![Data::Recent(2)]).is_empty());
    }
}
