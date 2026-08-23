//! Streaming a message or one MIME part into a [`BodySink`] — no whole body
//! ever held in memory.
//!
//! # Two streaming strategies, chosen by `BodyPart`
//!
//! `io-imap` 0.6.0 ships a real byte-streaming FETCH
//! (`rfc3501::fetch_stream::ImapMessageFetchStream`), but it is hard-coded to
//! `BODY.PEEK[]` — the *whole* message, with no section parameter. There is
//! no public API to stream an individual MIME part that way, which is
//! exactly the case that matters most for memory: a large attachment is a
//! section, not a whole message.
//!
//! So [`BodyPart::Whole`] drives that real streaming coroutine directly
//! against the session's transport (see [`stream_whole`]) — one round trip,
//! bytes handed to the sink as they arrive off the socket, regardless of
//! message size. [`BodyPart::Headers`], [`BodyPart::Text`] and
//! [`BodyPart::Section`] have no such primitive available, so they fall back
//! to a loop of bounded RFC 3501 §6.4.5 partial fetches instead (see
//! [`stream_windows`]): `BODY.PEEK[section]<offset.count>`, one
//! [`PARTIAL_FETCH_WINDOW`]-sized round trip at a time, until the server
//! returns less than a full window. That costs more round trips than true
//! streaming for a large part, but it is the only way to bound memory when
//! the section itself cannot be named to the streaming coroutine.
//!
//! A cancelled or failed fetch never calls [`BodySink::finish`]: whatever
//! partial bytes reached the sink are the caller's to discard, per the
//! sink's own contract. Retrying is starting over, not resuming a byte
//! offset — this module does not track one across calls.

use std::num::NonZeroU32;

use io_imap::client::ImapClientAsync;
use io_imap::coroutine::{ImapCoroutine, ImapCoroutineState};
use io_imap::rfc3501::fetch::ImapMessageFetchOptions;
use io_imap::rfc3501::fetch_stream::{
    ImapMessageFetchStream, ImapMessageFetchStreamError, ImapMessageFetchStreamYield,
};
use io_imap::types::fetch::{MacroOrMessageDataItemNames, MessageDataItem, MessageDataItemName};
use io_imap::types::fetch::{Part, Section};
use io_imap::types::sequence::SequenceSet;
use postio_model::Uid;

use crate::backend::{BackendError, BackendResult, BodyPart, BodySink, FetchedBody};
use crate::cancel::CancelToken;

use super::{ConnectionPool, ImapSession, Priority, READ_BUFFER, TransportError};

/// Octets asked for per round trip. See the [module docs](self) for why this
/// is a loop of partial fetches rather than one streamed command.
pub const PARTIAL_FETCH_WINDOW: u32 = 128 * 1024;

/// Streams `part` of `uid` in `mailbox` into `sink`.
pub async fn fetch_part(
    pool: &ConnectionPool,
    mailbox: &str,
    uid: Uid,
    part: &BodyPart,
    sink: &mut dyn BodySink,
    priority: Priority,
    cancel: &CancelToken,
) -> BackendResult<FetchedBody> {
    if cancel.is_cancelled() {
        return Err(BackendError::Cancelled);
    }

    let section = section_for(part)?;
    let mailbox_owned = mailbox.to_owned();

    let bytes_written = pool
        .execute(priority, async |session| {
            session.ensure_selected(&mailbox_owned, false).await?;
            if matches!(part, BodyPart::Whole) {
                stream_whole(session, &mailbox_owned, uid, sink, cancel).await
            } else {
                stream_windows(session, &mailbox_owned, uid, &section, sink, cancel).await
            }
        })
        .await?;

    sink.finish().await?;

    Ok(FetchedBody {
        uid,
        part: part.clone(),
        bytes_written,
    })
}

/// Fetches `section` in [`PARTIAL_FETCH_WINDOW`]-sized windows, writing each
/// to `sink` as it arrives, until the server returns less than a full
/// window. Does not call [`BodySink::finish`] — that is [`fetch_part`]'s job,
/// once every window succeeded.
async fn stream_windows(
    session: &mut ImapSession,
    mailbox: &str,
    uid: Uid,
    section: &Option<Section<'static>>,
    sink: &mut dyn BodySink,
    cancel: &CancelToken,
) -> BackendResult<u64> {
    let window = NonZeroU32::new(PARTIAL_FETCH_WINDOW).expect("PARTIAL_FETCH_WINDOW is nonzero");
    let mut offset: u32 = 0;
    let mut total: u64 = 0;

    loop {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        let bytes = fetch_window(session, mailbox, uid, section, offset, window).await?;
        let len = bytes.len();

        if !bytes.is_empty() {
            sink.chunk(&bytes).await?;
            total += len as u64;
            offset += u32::try_from(len).unwrap_or(u32::MAX);
        }

        if len < PARTIAL_FETCH_WINDOW as usize {
            break;
        }
    }

    Ok(total)
}

/// Streams the whole message in one round trip via `io-imap`'s real
/// streaming `UID FETCH … BODY.PEEK[]` coroutine.
///
/// Unlike [`stream_windows`] this drives the coroutine directly against the
/// session's transport rather than going through [`ImapClientAsync`]: the
/// coroutine's own yield vocabulary (`BodyChunk`, `WantsStream`) is one of
/// the handful the crate documents as meant to be wired per-caller, the same
/// reason `ImapSession::open` pumps `ImapSessionOpen` by hand rather than
/// through `run`.
async fn stream_whole(
    session: &mut ImapSession,
    mailbox: &str,
    uid: Uid,
    sink: &mut dyn BodySink,
    cancel: &CancelToken,
) -> BackendResult<u64> {
    let id = NonZeroU32::new(uid.get()).ok_or_else(|| BackendError::Protocol {
        reason: "UID 0 cannot be fetched".to_owned(),
    })?;

    let mut coroutine = ImapMessageFetchStream::new(id, true);
    let mut buffer = [0u8; READ_BUFFER];
    let mut resume: Option<&[u8]> = None;
    let mut total: u64 = 0;
    // `ImapMessageFetchStream` completes `Ok(())` both when the server sent
    // no FETCH data at all (the UID does not exist — RFC 3501 says a FETCH
    // of a missing id is a bare tagged OK) and when it streamed a genuine
    // body, and does not distinguish the two itself. This is the only signal
    // available to tell them apart from here.
    let mut saw_body = false;

    loop {
        if cancel.is_cancelled() {
            return Err(BackendError::Cancelled);
        }

        match coroutine.resume(&mut session.fragmentizer, resume.take()) {
            ImapCoroutineState::Complete(Ok(())) => break,
            ImapCoroutineState::Complete(Err(error)) => {
                return Err(map_fetch_stream_error(error));
            }
            ImapCoroutineState::Yielded(ImapMessageFetchStreamYield::WantsWrite(bytes)) => {
                session.stream.write_all(&bytes).await?;
            }
            ImapCoroutineState::Yielded(ImapMessageFetchStreamYield::WantsRead) => {
                // Bounded on silence, not on how long the download takes: a
                // large attachment over a slow link is not a hung server.
                let timeout = session.command_timeout();
                let read = session
                    .stream
                    .read_within(&mut buffer, timeout, "a streamed FETCH")
                    .await?;
                resume = Some(&buffer[..read]);
            }
            ImapCoroutineState::Yielded(ImapMessageFetchStreamYield::BodyChunk(bytes)) => {
                saw_body = true;
                total += bytes.len() as u64;
                sink.chunk(&bytes).await?;
            }
            ImapCoroutineState::Yielded(ImapMessageFetchStreamYield::WantsStream { len }) => {
                saw_body = true;
                let (written, short) =
                    stream_len_into_sink(session, sink, len, &mut buffer).await?;
                total += written;
                // The coroutine's own resume convention for this step: `None`
                // reports that every requested octet reached the sink,
                // `Some(&[])` reports a short read — never the bytes
                // themselves, which already went straight to the sink.
                resume = if short { Some(&[]) } else { None };
            }
        }
    }

    if !saw_body {
        return Err(BackendError::NoSuchMessage {
            mailbox: mailbox.to_owned(),
            uid: uid.get(),
        });
    }

    Ok(total)
}

/// Reads exactly `len` octets from `session`'s transport into `sink`, in
/// [`READ_BUFFER`]-sized chunks so a multi-megabyte message never exists
/// whole in memory. Returns the octets actually written and whether the
/// connection closed before all of `len` arrived.
async fn stream_len_into_sink(
    session: &mut ImapSession,
    sink: &mut dyn BodySink,
    mut len: u32,
    buffer: &mut [u8],
) -> BackendResult<(u64, bool)> {
    let mut written = 0u64;

    let timeout = session.command_timeout();
    while len > 0 {
        let want = (len as usize).min(buffer.len());
        match session
            .stream
            .read_within(&mut buffer[..want], timeout, "a streamed body")
            .await
        {
            Ok(read) => {
                sink.chunk(&buffer[..read]).await?;
                written += read as u64;
                len -= u32::try_from(read).unwrap_or(len);
            }
            Err(TransportError::Closed) => return Ok((written, true)),
            Err(other) => return Err(other.into()),
        }
    }

    Ok((written, false))
}

fn map_fetch_stream_error(error: ImapMessageFetchStreamError) -> BackendError {
    match error {
        ImapMessageFetchStreamError::No(reason) | ImapMessageFetchStreamError::Bad(reason) => {
            BackendError::Rejected {
                command: "FETCH".to_owned(),
                reason,
            }
        }
        ImapMessageFetchStreamError::Bye(reason) => BackendError::Disconnected {
            context: "the IMAP session".to_owned(),
            reason,
        },
        // The socket died in the middle of the exchange: the literal stopped
        // short of the octet count the server announced, or the tagged
        // response never came. That is the connection failing, not the server
        // refusing — and the difference decides whether the caller retries,
        // because `Protocol` is permanent and `Disconnected` is not. Found by
        // `tests/imap_loopback.rs`, which tears a connection mid-FETCH.
        error @ (ImapMessageFetchStreamError::ShortBody
        | ImapMessageFetchStreamError::MissingTagged) => BackendError::Disconnected {
            context: "a streamed FETCH".to_owned(),
            reason: error.to_string(),
        },
        other => BackendError::Protocol {
            reason: other.to_string(),
        },
    }
}

/// One `UID FETCH … BODY.PEEK[section]<offset.window>` round trip.
async fn fetch_window(
    session: &mut ImapSession,
    mailbox: &str,
    uid: Uid,
    section: &Option<Section<'static>>,
    offset: u32,
    window: NonZeroU32,
) -> BackendResult<Vec<u8>> {
    let sequence_set =
        SequenceSet::from(
            NonZeroU32::new(uid.get()).ok_or_else(|| BackendError::Protocol {
                reason: "UID 0 cannot be fetched".to_owned(),
            })?,
        );
    let items =
        MacroOrMessageDataItemNames::MessageDataItemNames(vec![MessageDataItemName::BodyExt {
            section: section.clone(),
            partial: Some((offset, window)),
            peek: true,
        }]);
    let opts = ImapMessageFetchOptions {
        uid: true,
        modifiers: Vec::new(),
    };

    let raw = session.fetch(sequence_set, items, opts).await;
    let raw = raw.map_err(|error| session.command_error("FETCH", error))?;

    let Some(items) = raw.into_values().next() else {
        return Err(BackendError::NoSuchMessage {
            mailbox: mailbox.to_owned(),
            uid: uid.get(),
        });
    };

    for item in items {
        if let MessageDataItem::BodyExt { data, .. } = item {
            return Ok(data
                .into_option()
                .map(|bytes| bytes.into_owned())
                .unwrap_or_default());
        }
    }
    Ok(Vec::new())
}

fn section_for(part: &BodyPart) -> BackendResult<Option<Section<'static>>> {
    Ok(match part {
        BodyPart::Whole => None,
        BodyPart::Headers => Some(Section::Header(None)),
        BodyPart::Text => Some(Section::Text(None)),
        BodyPart::Section(spec) => Some(Section::Part(parse_part_number(spec)?)),
    })
}

/// Parses an IMAP section number like `2.1` into the dotted `Part` it names.
fn parse_part_number(spec: &str) -> BackendResult<Part> {
    let malformed = || BackendError::Protocol {
        reason: format!("{spec:?} is not a valid IMAP body section number"),
    };

    let numbers: Vec<NonZeroU32> = spec
        .split('.')
        .map(|piece| piece.parse::<u32>().ok().and_then(NonZeroU32::new))
        .collect::<Option<_>>()
        .ok_or_else(malformed)?;

    Ok(Part(imap_types_vec1(numbers).ok_or_else(malformed)?))
}

fn imap_types_vec1(numbers: Vec<NonZeroU32>) -> Option<io_imap::types::core::Vec1<NonZeroU32>> {
    io_imap::types::core::Vec1::try_from(numbers).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_has_no_section() {
        assert_eq!(section_for(&BodyPart::Whole).unwrap(), None);
    }

    #[test]
    fn headers_and_text_map_to_their_named_sections() {
        assert!(matches!(
            section_for(&BodyPart::Headers).unwrap(),
            Some(Section::Header(None))
        ));
        assert!(matches!(
            section_for(&BodyPart::Text).unwrap(),
            Some(Section::Text(None))
        ));
    }

    #[test]
    fn a_dotted_section_number_parses_into_its_parts() {
        let section = section_for(&BodyPart::section("2.1")).unwrap();
        let Some(Section::Part(Part(numbers))) = section else {
            panic!("expected Section::Part");
        };
        let numbers: Vec<u32> = numbers.into_iter().map(NonZeroU32::get).collect();
        assert_eq!(numbers, vec![2, 1]);
    }

    #[test]
    fn a_malformed_section_number_is_rejected_not_defaulted() {
        let error = section_for(&BodyPart::section("2.x")).unwrap_err();
        assert!(error.to_string().contains("2.x"));
    }

    #[test]
    fn an_empty_section_number_is_rejected() {
        assert!(section_for(&BodyPart::section("")).is_err());
    }
}
