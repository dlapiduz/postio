//! Streaming a message or one MIME part into a [`BodySink`] — no whole body
//! ever held in memory.
//!
//! # Why partial fetch instead of `io-imap`'s streaming coroutine
//!
//! `io-imap` 0.6.0 ships a real byte-streaming FETCH
//! (`rfc3501::fetch_stream`), but it is hard-coded to `BODY.PEEK[]` — the
//! *whole* message, with no section parameter. There is no public API to
//! stream an individual MIME part, which is exactly the case that matters
//! most here: a large attachment is a section, not a whole message.
//!
//! So this fetches in bounded windows instead, using RFC 3501 §6.4.5's
//! partial fetch: `BODY.PEEK[section]<offset.count>`. Each round trip asks
//! for at most [`PARTIAL_FETCH_WINDOW`] octets and hands them to the sink; a
//! response shorter than the window means the part is exhausted. This costs
//! more round trips than true streaming would for a large part, but it
//! bounds memory to one window regardless of the part's size and works
//! uniformly for [`BodyPart::Whole`], [`BodyPart::Headers`],
//! [`BodyPart::Text`] and [`BodyPart::Section`] alike — `io-imap`'s
//! non-streaming `FETCH` already decodes all four the same way. Adopting the
//! real streaming coroutine for the [`BodyPart::Whole`] case specifically,
//! to cut round trips on a full raw-message download, is a reasonable
//! follow-up and is filed as one rather than blocking this bead on it.
//!
//! A cancelled or failed fetch never calls [`BodySink::finish`]: whatever
//! partial bytes reached the sink are the caller's to discard, per the
//! sink's own contract. Retrying is starting over, not resuming a byte
//! offset — this module does not track one across calls.

use std::num::NonZeroU32;

use io_imap::client::ImapClientAsync;
use io_imap::rfc3501::fetch::ImapMessageFetchOptions;
use io_imap::types::fetch::{MacroOrMessageDataItemNames, MessageDataItem, MessageDataItemName};
use io_imap::types::fetch::{Part, Section};
use io_imap::types::sequence::SequenceSet;
use postio_model::Uid;

use crate::backend::{BackendError, BackendResult, BodyPart, BodySink, FetchedBody};
use crate::cancel::CancelToken;

use super::{ConnectionPool, ImapSession, Priority, map_client_error};

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
            stream_windows(session, &mailbox_owned, uid, &section, sink, cancel).await
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

    let raw = session
        .fetch(sequence_set, items, opts)
        .await
        .map_err(|error| map_client_error("FETCH", session.account(), error))?;

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
