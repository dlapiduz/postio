//! The wire between a frontend and the store's one owner.
//!
//! `specs/005-tui-frontend/contracts/protocol.md` is the contract; this is its
//! types and its framing. Client and host always come from one build, so there
//! is no negotiation: a handshake that finds another build is refused, and the
//! frontend says which two versions disagree.
//!
//! **Nothing here waits on the network.** Every [`Req`] is answered from the
//! host's local state; anything remote is a [`Command`], whose outcome arrives
//! later as events (Principle I).
//!
//! # Framing
//!
//! A frame is a big-endian `u32` length followed by that many bytes of JSON.
//! JSON rather than a compact binary format because several model types
//! deserialize by hand, and a format that is not self-describing cannot
//! always read those back. The cost is measured in microseconds for a page of
//! rows (research R2). [`MAX_FRAME`] refuses a length no real frame has, so
//! a corrupt stream is an error rather than a gigabyte allocation.

use postio_core::{Command, EventEnvelope, InvocationId, StateSnapshot};
use postio_model::Account;
use postio_model::ListScope;
use postio_model::ids::{AccountId, MailboxId, MessageId};
use postio_model::listing::{
    DraftCounts, ListPage, ListRows, MessageSummary, PageRequest, StoreError,
};
use postio_model::mailbox::Mailbox;
use serde::{Deserialize, Serialize};

/// The protocol's own version. Bumped with any change to [`Frame`]; the
/// build id already refuses a mismatch, and this makes the refusal's reason
/// legible.
pub const PROTOCOL: u32 = 1;

/// The largest frame either side will read: 256 MiB. A message body is the
/// largest thing that crosses, and the store refuses bodies far below this.
pub const MAX_FRAME: u32 = 256 * 1024 * 1024;

/// Which build a process is: the crate version and, when the build knows
/// it, the commit. Client and host must agree exactly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildId(pub String);

impl BuildId {
    /// This build.
    pub fn current() -> Self {
        let version = env!("CARGO_PKG_VERSION");
        match option_env!("POSTIO_GIT_COMMIT") {
            Some(commit) => BuildId(format!("{version}+{commit}")),
            None => BuildId(version.to_owned()),
        }
    }
}

impl std::fmt::Display for BuildId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// What kind of frontend is connecting. Decides who delivers notifications
/// and labels the connection in the host's log.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ClientKind {
    /// The GTK desktop app.
    Gtk,
    /// `postio-tui`.
    Tui,
    /// The macOS frontend, through `postio-ffi`.
    Ffi,
    /// A test.
    Test,
}

/// The host's name for one connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ClientId(pub u64);

/// Why the host would not take a connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Refusal {
    /// The two sides are different builds.
    VersionMismatch {
        /// The host's build.
        host: BuildId,
        /// The client's build.
        client: BuildId,
    },
    /// The host is still opening the store; try again shortly.
    Starting,
}

/// A request a frontend makes of the host. Each is answered by exactly one
/// [`Resp`] with the same id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Req {
    /// Run a command, aimed with the frontend's own selection; its effects
    /// arrive as events.
    Send(Command, StateSnapshot),
    /// The same, learning the id its events will carry.
    SendTracked(Command, StateSnapshot),
    /// One page of a list.
    Page(PageRequest),
    /// How many rows a list would show.
    Count(ListScope),
    /// The rows for these messages, in the order given.
    Rows(Vec<MessageId>),
    /// The rows a scope's list shows for these messages.
    RowsIn(ListScope, Vec<MessageId>),
    /// These messages left a mailbox; said before the list re-reads.
    NoteRemoved(MailboxId, Vec<MessageId>),
    /// An account's folders.
    Mailboxes(AccountId),
    /// What the sidebar draws beside Drafts and the Outbox.
    DraftCounts(AccountId),
    /// Every account, in the sidebar's order.
    Accounts,
    /// A message's body, or why there is none yet.
    Body(MessageId),
    /// A conversation's messages, oldest first, as list rows.
    Conversation(postio_model::ThreadId),
    /// Leave the list this message came from: record the activation, and
    /// answer the list's name. Only ever on a person's deliberate act.
    Unsubscribe(MessageId),
    /// A message's parts: its attachments, and inline parts.
    Parts(MessageId),
    /// Write one part to `to`, fetching it first if it was never downloaded.
    SavePart {
        /// Whose.
        message: MessageId,
        /// Which part.
        attachment: postio_model::ids::AttachmentId,
        /// Where; the frontend chose it.
        to: std::path::PathBuf,
    },
    /// Write one part to a private temporary file for the system to open,
    /// and answer where.
    OpenPart {
        /// Whose.
        message: MessageId,
        /// Which part.
        attachment: postio_model::ids::AttachmentId,
    },
}

/// The host's answer to one [`Req`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Resp {
    /// Done; nothing to report.
    Done,
    /// A tracked send was queued; its events carry this id.
    Tracked(InvocationId),
    /// The runtime has stopped and took no command.
    Stopped,
    /// A page.
    Page(ListPage),
    /// A count.
    Count(u32),
    /// Rows.
    Rows(Vec<MessageSummary>),
    /// Rows, in the scope's shape.
    ListRows(ListRows),
    /// Folders.
    Mailboxes(Vec<Mailbox>),
    /// Draft counts.
    DraftCounts(DraftCounts),
    /// The accounts.
    Accounts(Vec<Account>),
    /// A body.
    Body(Body),
    /// The list a message was unsubscribed from.
    Unsubscribed(String),
    /// A message's parts.
    Parts(Vec<postio_model::Attachment>),
    /// Where a part was written.
    Saved(std::path::PathBuf),
    /// The read could not be answered; the sentence is for the user.
    Failed(StoreError),
}

/// A message's body as the store holds it, or which kind of "no body" this
/// is. The frontend applies the reader's rules to it -- sanitising, reader
/// view, folding -- with the same shared code every reader uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Body {
    /// The words are here.
    Ready {
        /// The text and HTML parts.
        body: postio_model::MessageBody,
        /// Whether those words are a guess rather than what was sent.
        encoding_problems: bool,
    },
    /// Headers are here; the body has not been fetched yet.
    Partial,
    /// Not fetched, and nothing is fetching: offline.
    Offline,
    /// Recorded, but the bytes are not in the store.
    Missing,
    /// Fetched, and there is no text or HTML part.
    Empty,
    /// A draft written by another client: nothing here to edit.
    ForeignDraft,
}

/// Everything that crosses the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frame {
    /// The client's first frame.
    Hello {
        /// The client's build.
        build: BuildId,
        /// What kind of frontend it is.
        kind: ClientKind,
        /// [`PROTOCOL`] as the client knows it.
        protocol: u32,
    },
    /// The host took the connection.
    Welcome {
        /// This connection's id.
        client: ClientId,
        /// The host's build, equal to the client's.
        host_build: BuildId,
    },
    /// The host would not take the connection.
    Refused(Refusal),
    /// A request.
    Request {
        /// Echoed on the response.
        id: u64,
        /// What is asked.
        body: Req,
    },
    /// The answer to the request with the same id.
    Response {
        /// The request's id.
        id: u64,
        /// The answer.
        body: Resp,
    },
    /// Something happened; sent unasked, in order.
    Event(EventEnvelope),
}

/// A frame that could not be read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    /// The length prefix names more bytes than any frame may hold.
    #[error("a frame of {0} bytes is larger than any Postio frame")]
    TooLarge(u32),
    /// Fewer bytes than the length prefix promised.
    #[error("the frame ended early")]
    Truncated,
    /// The bytes are not a frame.
    #[error("the frame could not be read: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// `frame`, length-prefixed, ready to write.
pub fn encode(frame: &Frame) -> Vec<u8> {
    // Serialising our own types cannot fail: every map key is a string and
    // nothing implements `Serialize` fallibly.
    let body = serde_json::to_vec(frame).expect("a frame always serialises");
    let length = u32::try_from(body.len()).expect("no frame reaches 4 GiB");
    let mut bytes = Vec::with_capacity(4 + body.len());
    bytes.extend_from_slice(&length.to_be_bytes());
    bytes.extend_from_slice(&body);
    bytes
}

/// One frame from the front of `bytes`, and how many bytes it took; `None`
/// when `bytes` does not yet hold a whole frame.
pub fn decode(bytes: &[u8]) -> Result<Option<(Frame, usize)>, FrameError> {
    let Some(prefix) = bytes.first_chunk::<4>() else {
        return Ok(None);
    };
    let length = u32::from_be_bytes(*prefix);
    if length > MAX_FRAME {
        return Err(FrameError::TooLarge(length));
    }
    let end = 4 + length as usize;
    let Some(body) = bytes.get(4..end) else {
        return Ok(None);
    };
    Ok(Some((serde_json::from_slice(body)?, end)))
}

/// Read one frame from `reader`; `None` at a clean end of stream.
pub async fn read_frame<R>(reader: &mut R) -> Result<Option<Frame>, FrameError>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;
    let mut prefix = [0u8; 4];
    match reader.read_exact(&mut prefix).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(_) => return Err(FrameError::Truncated),
    }
    let length = u32::from_be_bytes(prefix);
    if length > MAX_FRAME {
        return Err(FrameError::TooLarge(length));
    }
    let mut body = vec![0u8; length as usize];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|_| FrameError::Truncated)?;
    Ok(Some(serde_json::from_slice(&body)?))
}

/// Write one frame to `writer`.
pub async fn write_frame<W>(writer: &mut W, frame: &Frame) -> std::io::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    writer.write_all(&encode(frame)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::Event;

    fn round_trip(frame: Frame) {
        let bytes = encode(&frame);
        let (back, used) = decode(&bytes)
            .expect("a frame we wrote is readable")
            .expect("and whole");
        assert_eq!(used, bytes.len(), "the whole frame was consumed");
        assert_eq!(back, frame);
    }

    #[test]
    fn a_handshake_round_trips() {
        round_trip(Frame::Hello {
            build: BuildId::current(),
            kind: ClientKind::Tui,
            protocol: PROTOCOL,
        });
        round_trip(Frame::Refused(Refusal::VersionMismatch {
            host: BuildId("0.3.0+aaa".into()),
            client: BuildId("0.3.0+bbb".into()),
        }));
    }

    #[test]
    fn every_request_round_trips() {
        let account = AccountId::new(1);
        let mailbox = MailboxId::new(2);
        let messages = vec![MessageId::new(3), MessageId::new(4)];
        let scope = ListScope::Mailbox(mailbox);
        for (id, body) in [
            Req::Send(
                Command::Archive {
                    target: postio_core::MessageTarget::Messages(messages.clone()),
                },
                StateSnapshot::default(),
            ),
            Req::SendTracked(Command::Undo, StateSnapshot::default()),
            Req::Page(PageRequest {
                scope,
                offset: 40,
                limit: 20,
            }),
            Req::Count(scope),
            Req::Rows(messages.clone()),
            Req::RowsIn(scope, messages.clone()),
            Req::NoteRemoved(mailbox, messages.clone()),
            Req::Mailboxes(account),
            Req::DraftCounts(account),
        ]
        .into_iter()
        .enumerate()
        {
            round_trip(Frame::Request {
                id: id as u64,
                body,
            });
        }
    }

    #[test]
    fn answers_and_events_round_trip() {
        round_trip(Frame::Response {
            id: 7,
            body: Resp::DraftCounts(DraftCounts {
                outbox: 1,
                attention: 2,
                drafts: 3,
            }),
        });
        round_trip(Frame::Response {
            id: 8,
            body: Resp::Failed(StoreError::new("The store is closed.")),
        });
        round_trip(Frame::Event(EventEnvelope::untracked(
            Event::MailboxesChanged {
                account: AccountId::new(1),
            },
        )));
    }

    #[test]
    fn a_partial_frame_is_not_yet_a_frame() {
        let bytes = encode(&Frame::Refused(Refusal::Starting));
        assert!(bytes.len() > 4, "a frame has a body: {bytes:?}");
        assert!(
            decode(&bytes[..bytes.len() - 1])
                .expect("not an error")
                .is_none()
        );
        assert!(decode(&bytes[..2]).expect("not an error").is_none());
    }

    #[test]
    fn a_length_no_frame_has_is_refused_before_reading_it() {
        let mut bytes = (MAX_FRAME + 1).to_be_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        assert!(matches!(decode(&bytes), Err(FrameError::TooLarge(_))));
    }
}
