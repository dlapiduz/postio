//! A message's parts, so an attachment can be saved.
//!
//! The one surface whose absence made a whole category of mail unusable on
//! macOS: an attachment that arrived there could not be saved by any route,
//! because there was no parts panel and no call under one. #1572.
//!
//! # What crosses, and what does not
//!
//! Everything that is a *decision* crosses already made:
//! [`postio_ui::reader::parts`] works out the tree from the MIME paths, what
//! each row is called, what each state's sentence is, and — above all — the
//! filename a part may be written under. The frontend receives words and
//! bytes and decides none of it.
//!
//! What does not cross is where the keyboard is. A parts panel's cursor is
//! the panel's own state, the same way a text field's insertion point is;
//! what is shared is the *rule* — [`part_cursor_after`] — so the two
//! frontends walk one tree the same way rather than each inventing a wrap or
//! a clamp. `next_part`, `prev_part` and `open_parts` therefore have no
//! method here at all: they move or create a surface this side does not own.
//!
//! # Handing a file to another application is a consent moment
//!
//! `ARCHITECTURE.md` §11 turns on "did the user ask for it", and three of
//! these calls put the user's mail somewhere another program can read it. So
//! the boundary is arranged to make the asking explicit rather than to trust
//! that it happened:
//!
//! - **Listing fetches nothing.** [`Session::message_parts`] reads the rows a
//!   sync already wrote from `BODYSTRUCTURE` and has no route to the bytes.
//!   Drawing a panel cannot touch the network, so a panel cannot be the thing
//!   that leaks.
//! - **Only a named part is fetched**, one call per deliberate act, and never
//!   speculatively. There is no "prefetch the attachments of the open
//!   message" call here and there must not be one.
//! - **Postio names the file on every path that hands it over.**
//!   [`Session::export_part`] — what "Open with…" and a drag-out are built on
//!   — takes a *directory* and nothing else, so a frontend cannot pass the
//!   sender's `filename=` through to the filesystem even by accident.
//! - **Nothing here launches anything.** The `NSWorkspace` call is Swift's,
//!   under the `POSTIO-CONSENT:` comment `check-no-silent-tracking.py`
//!   requires, because the platform's own launcher is the platform's to call
//!   — but also because the refusal should be visible in the one place a
//!   reviewer looks for it.
//! - **Logs carry the part id, the byte count and the outcome.** Never the
//!   filename: the sender chose it, and it is as much the user's mail as the
//!   body is.
//!
//! # Listing is instant; getting bytes is not
//!
//! [`Session::message_parts`] is a bounded indexed read and answers in
//! milliseconds, which is why a panel can be drawn straight from a keypress.
//! The other four — `partBytes`, `savePart`, `exportPart`, `saveAllParts` —
//! are the opposite, and it is worth saying plainly because they look the
//! same from Swift: they are synchronous, they may fetch, and a fetch is
//! *waited on* for up to thirty seconds before it gives up.
//! `saveAllParts` does that once per part, in sequence.
//!
//! So those four must never be called from the main actor. That is the same
//! contract `syncNow` and `accounts` carry and for the same reason — the
//! exported surface is synchronous because Swift asked for a value rather
//! than a task, and what pays for it is the caller putting the call in one.
//! `postio_session::blocking`'s own module docs are blunt about it: nothing
//! that could take real time may come through that bridge on a thread that
//! is drawing.

/// One node of a message's MIME tree, ready to draw.
///
/// A flattened tree rather than a nested one: the frontend draws a list, the
/// walk order is the list order, and `depth` plus `prefix` carry the shape.
/// A nested record would make the frontend flatten it again to draw it, and
/// two flattenings is two walk orders.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PartFfi {
    /// The IMAP part id — `2.1`. Empty for the message itself.
    ///
    /// **This is the handle**, and it is a MIME path rather than a row id on
    /// purpose: a whole-message fetch replaces a message's attachment rows,
    /// so an `AttachmentId` held across one stops meaning anything, while `2`
    /// is `2` in every parse of the same bytes.
    pub part_id: String,
    /// How deep it sits; the message itself is 0.
    pub depth: u32,
    /// The box-drawing prefix down the left of the tree — `  ├ `.
    ///
    /// Shared rather than derived from `depth`, because which branch
    /// character a row gets depends on whether anything follows it *at its
    /// own level*, which a frontend holding one row cannot see.
    pub prefix: String,
    /// `text/html`, `image/png`, `multipart/mixed`.
    pub mime_type: String,
    /// The name the sender gave it, exactly as they wrote it, or `None`.
    ///
    /// **For display only.** It is attacker-controlled text and must never
    /// reach the filesystem; [`PartFfi::save_name`] is the one that may.
    pub filename: Option<String>,
    /// What the row is called: the filename, or the type when unnamed.
    pub label: String,
    /// A filename safe to write this part under, separators and control
    /// characters already taken out.
    pub save_name: String,
    /// Size in bytes as the server declared it. `0` for a container.
    pub size: u64,
    /// `image/png · 4.0 MB` — the type and the size, for a detail pane.
    pub detail: String,
    /// How the row reads to a screen reader.
    pub spoken: String,
    /// Whether the bytes are already on this machine.
    ///
    /// `false` is the ordinary state of an attachment on a message that has
    /// only been described, not a fault — see the module docs.
    pub downloaded: bool,
    /// Whether this holds other parts rather than bytes of its own. Nothing
    /// to save, and nothing to open.
    pub is_container: bool,
    /// Whether the message means this part to be shown in place — a `cid:`
    /// the body references, or a `Content-Disposition: inline`.
    ///
    /// The distinction a frontend must not guess at: a list that called the
    /// sender's signature logo an attachment would offer it beside their
    /// invoice.
    pub inline: bool,
    /// The `Content-ID` the body may reference this part by, if it has one.
    ///
    /// What a `postio-cid:` scheme handler resolves against —
    /// `Session::resolveCid` takes exactly this, scoped to this message.
    pub content_id: Option<String>,
    /// Whether Postio can show this part itself rather than only save it.
    ///
    /// Images and PDFs. Everything else is bytes the application has no
    /// business interpreting, and "open" on one means handing it to the
    /// desktop rather than guessing.
    pub previewable: bool,
}

impl From<&postio_ui::reader::parts::Node> for PartFfi {
    fn from(node: &postio_ui::reader::parts::Node) -> Self {
        use postio_ui::reader::parts as shared;
        PartFfi {
            part_id: node.part_id.clone(),
            depth: node.depth as u32,
            prefix: shared::prefix(node),
            mime_type: node.mime.clone(),
            filename: node.filename.clone(),
            label: node.label().to_string(),
            save_name: shared::save_name(node),
            size: node.size,
            detail: shared::detail(node),
            spoken: shared::spoken(node),
            downloaded: node.downloaded,
            is_container: !node.is_leaf(),
            inline: node.inline,
            content_id: node.content_id.clone(),
            previewable: shared::previewable(&node.mime),
        }
    }
}

impl From<&PartFfi> for postio_ui::reader::parts::Node {
    /// Back to a node, for the rules that take one.
    ///
    /// Lossy in one direction that does not matter and one that does. `last`
    /// is not carried — the prefix it decides has already been rendered — and
    /// neither is the attachment row id, because nothing on this side of the
    /// boundary may address a part by one (see [`PartFfi::part_id`]). What is
    /// carried is everything the note and the name depend on, which is what
    /// keeps a round trip from changing either.
    fn from(part: &PartFfi) -> Self {
        postio_ui::reader::parts::Node {
            part_id: part.part_id.clone(),
            depth: part.depth as usize,
            mime: part.mime_type.clone(),
            filename: part.filename.clone(),
            size: part.size,
            downloaded: part.downloaded,
            last: true,
            // `is_leaf` reads this and the mime type together, so a container
            // has to come back without a row to stand on or it would read as
            // something with bytes to save.
            attachment: (!part.is_container).then(|| postio_model::ids::AttachmentId::new(0)),
            content_id: part.content_id.clone(),
            inline: part.inline,
        }
    }
}

/// A message's whole MIME tree, and where a panel opens on it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessagePartsFfi {
    /// The message's own content type — `multipart/mixed`.
    pub root: String,
    /// The header line: `multipart/mixed · 4 parts · 1.2 MB`.
    pub summary: String,
    /// Every node in walk order. The first is the message itself, which is
    /// why a tree is never empty: one row saying `text/plain · 0 parts` is a
    /// true answer where a blank panel looks broken.
    pub parts: Vec<PartFfi>,
    /// Which row the cursor starts on — the first part, not the message.
    ///
    /// Opening on the container would cost a keystroke every single time. The
    /// frontend owns the cursor from here; this is only where to put it.
    pub cursor: u32,
}

/// How a "save every part" went.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct SavedPartsFfi {
    /// How many landed on disk.
    pub saved: u32,
    /// How many could not be had — still on a server that would not answer,
    /// most likely. The batch never abandons the rest for one of these.
    pub failed: u32,
    /// One sentence for the whole batch, or `None` when nothing failed.
    ///
    /// A sentence rather than a count to phrase, so the two frontends report
    /// the same partial save the same way. One message for the batch rather
    /// than one per part: "save all" can easily name a dozen.
    pub failure: Option<String>,
}

/// Why a part could not be had, or could not be written.
///
/// One variant carrying the sentence, the shape [`crate::ComposeError`] uses
/// and for the same reason: the frontend's job is to show this to the person
/// who pressed save, not to branch on it. A save that quietly produced
/// nothing is the failure being avoided — a zero-byte file on disk looks like
/// a saved attachment and is not one.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PartsError {
    /// It could not be done, and this is why in words.
    #[error("{message}")]
    Refused {
        /// What went wrong.
        message: String,
    },
}

impl From<String> for PartsError {
    fn from(message: String) -> Self {
        PartsError::Refused { message }
    }
}

/// Where the cursor goes when the walk keys are pressed.
///
/// The rule, not the state. `count` is the number of rows including the
/// message itself — `MessagePartsFfi::parts.count`.
///
/// **Clamps rather than wraps**: a tree is read downwards, and a `j` at the
/// bottom that jumped back to the header would lose the reader's place in a
/// structure they were working through.
#[uniffi::export]
pub fn part_cursor_after(current: u32, forward: bool, count: u32) -> u32 {
    postio_ui::reader::parts::step(current as usize, forward, count as usize) as u32
}

/// The sentence beside the part the cursor is on.
///
/// Four states with four different things to say, and a frontend that wrote
/// its own would be the frontend where #751's oversized inline image had no
/// explanation: the `cid:` resolves to nothing, the body draws a broken box,
/// and this line on that part's row is all the user gets.
///
/// The held-back counts are the reader's, for the message this part belongs
/// to — `0, 0` when nothing was held back.
#[uniffi::export]
pub fn part_note(part: PartFfi, remote_images: u32, trackers: u32) -> String {
    postio_ui::reader::parts::note(&(&part).into(), remote_images, trackers)
}

/// Why a part is being held back, or `None` when it is not.
///
/// What decides whether "Render once" is offered at all, and what to say
/// above it. Only the markup part is ever held back: an `image/png`
/// attachment references nothing and cannot phone home, so offering to render
/// *it* once would be theatre.
///
/// **Rendering once is not a grant.** The frontend answers this by asking for
/// the document again with `RemoteImagesFfi::Allowed` for that one message
/// and nothing else — it must not write an allowlist entry, or a key meaning
/// "just this once" would quietly mean "from now on".
#[uniffi::export]
pub fn part_held_back_note(mime_type: String, remote_images: u32, trackers: u32) -> Option<String> {
    postio_ui::reader::parts::held_back_note(&mime_type, remote_images, trackers)
}

impl MessagePartsFfi {
    /// A tree the store answered with, as the frontend sees it.
    pub(crate) fn from_parts(parts: postio_session::reading::MessageParts) -> Self {
        MessagePartsFfi {
            root: parts.root,
            summary: postio_ui::reader::parts::summary(&parts.nodes),
            cursor: postio_ui::reader::parts::first_part(parts.nodes.len()) as u32,
            parts: parts.nodes.iter().map(PartFfi::from).collect(),
        }
    }

    /// What a message nothing can be read for looks like.
    ///
    /// A store that is not open, or a message that has been deleted under the
    /// panel. Empty rather than an error because the caller is *drawing*: a
    /// panel with no rows is a truthful blank, where a thrown error would
    /// make listing the parts a thing that can fail and every frontend handle
    /// it.
    pub(crate) fn nothing() -> Self {
        MessagePartsFfi {
            root: String::new(),
            summary: String::new(),
            parts: Vec::new(),
            cursor: 0,
        }
    }
}
