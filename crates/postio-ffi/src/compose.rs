//! Writing mail, across a C ABI (#1272).
//!
//! What crosses is a **draft**, not an editor. Who a reply is addressed to,
//! what its subject becomes, what a quote looks like and which of my
//! identities it comes from are `postio_model::reply`'s answers — the same
//! ones the GTK composer gets — and the frontend's job is to show them and
//! take what the user types back.
//!
//! Addresses cross as text because that is what a person types into a `To`
//! field: `Ada Norwood <ada@example.com>, bo@example.com`. Parsing them is
//! `postio_model::address`'s, so the two frontends accept the same thing.

use postio_model::address;
use postio_model::draft::{Draft, DraftKind};
use postio_model::message::MessageBody;

/// What kind of composition a draft is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum DraftKindFfi {
    /// A message with no ancestor.
    New,
    /// A reply to its sender.
    Reply,
    /// A reply to everyone on it.
    ReplyAll,
    /// A forward.
    Forward,
}

impl From<DraftKind> for DraftKindFfi {
    fn from(kind: DraftKind) -> Self {
        match kind {
            DraftKind::New => Self::New,
            DraftKind::Reply => Self::Reply,
            DraftKind::ReplyAll => Self::ReplyAll,
            DraftKind::Forward => Self::Forward,
        }
    }
}

impl From<DraftKindFfi> for DraftKind {
    fn from(kind: DraftKindFfi) -> Self {
        match kind {
            DraftKindFfi::New => Self::New,
            DraftKindFfi::Reply => Self::Reply,
            DraftKindFfi::ReplyAll => Self::ReplyAll,
            DraftKindFfi::Forward => Self::Forward,
        }
    }
}

/// A message being written.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DraftFfi {
    /// The row this draft is, or `0` before it has been saved once.
    ///
    /// Carried back and forth because `save` is idempotent on it: a composer
    /// that forgot the id would insert a second row on every autosave, and
    /// the Drafts folder would fill with one half-written message.
    pub id: i64,
    /// The account it will be sent from.
    pub account: i64,
    /// What kind of composition this is.
    pub kind: DraftKindFfi,
    /// The address it comes from, rendered — `Mara Ostwald <mara@…>`.
    ///
    /// Read-only for now: choosing among an account's identities is its own
    /// surface, and a `From` field that could be edited into something the
    /// account cannot send as would be a lie the SMTP server corrects.
    pub from: String,
    /// `To`, as typed: `Ada Norwood <ada@example.com>, bo@example.com`.
    pub to: String,
    /// `Cc`, as typed.
    pub cc: String,
    /// `Bcc`, as typed.
    pub bcc: String,
    /// The subject line.
    pub subject: String,
    /// The body, as text.
    ///
    /// For a rich draft this is the `text/plain` alternative, derived from
    /// the document rather than typed: **rich mail sends `text/html` and a
    /// `text/plain` part always**, and a frontend that had to remember to
    /// build the second one would eventually send only the first. That is
    /// invisible to the sender and it is the whole message to a recipient
    /// reading in a terminal or with a screen reader.
    pub body: String,
    /// The body as marked-up text, when this draft is rich.
    ///
    /// Canonical, not "whatever the editing surface last had in its DOM":
    /// what crosses inbound is a working copy, and what is kept is what
    /// `postio_body::parse` makes of it (ADR 0004 Q3). So a `<div>` from a
    /// browser paste comes back as a paragraph and a `<script>` cannot be
    /// stored at all -- the type has no variant that could hold one.
    ///
    /// `None` for a plain draft. Not thrown away when the switch is turned
    /// off, though: the switch is on the document, and turning it back on
    /// should not have cost the marks.
    pub body_html: Option<String>,
    /// Whether this is being written as rich text.
    ///
    /// It decides what leaves: `text/html` plus a `text/plain` fallback
    /// **always**, against `text/plain` alone, wrapped and flowed. The
    /// composer says which in its footer, because what a mail client puts on
    /// the wire is not something a person should have to guess at.
    pub rich: bool,
    /// The message this answers, when it answers one.
    pub in_reply_to: Option<i64>,
    /// Where the draft lives on this disk, for the footer.
    pub path: String,
    /// What is attached to it.
    pub attachments: Vec<AttachmentFfi>,
}

/// One file attached to a draft.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AttachmentFfi {
    /// Its row, so one can be taken off again.
    pub id: i64,
    /// The name as it will arrive.
    pub filename: String,
    /// What it says it is.
    pub mime_type: String,
    /// How big, said the way a person thinks about it — `1.8 MB`.
    pub size: String,
}

/// What will be sent, in the composer footer's words: `html + text/plain`,
/// or `text/plain, format=flowed`.
///
/// A free function, not a method on the record. `#[uniffi::method]` inside a
/// `Record`'s `impl` compiles, generates nothing, and leaves a method that
/// works in Rust and does not exist in Swift — which is this repository's
/// characteristic bug wearing a boundary as a disguise.
#[uniffi::export]
pub fn outgoing_shape(rich: bool) -> String {
    postio_ui::compose::outgoing_shape(rich)
}

/// The draft as the boundary hands it over.
pub(crate) fn to_ffi(draft: &Draft, from: String, path: String) -> DraftFfi {
    DraftFfi {
        id: draft.id.get(),
        account: draft.account_id.into(),
        kind: draft.kind.into(),
        from,
        to: render(&draft.to),
        cc: render(&draft.cc),
        bcc: render(&draft.bcc),
        subject: draft.subject.clone(),
        body: draft.body.text.clone().unwrap_or_default(),
        body_html: draft.body.html.clone(),
        // The stored flag, not `html.is_some()` (#1271): a plain draft that
        // is keeping its marks in case the switch goes back on has an HTML
        // part and is not rich.
        rich: draft.rich,
        in_reply_to: draft.in_reply_to.map(|id| id.get()),
        path,
        attachments: draft
            .attachments
            .iter()
            .map(|attachment| AttachmentFfi {
                id: attachment.id.get(),
                // A file with no name is not nothing: it still arrives, and
                // saying so beats a blank row.
                filename: attachment
                    .filename
                    .clone()
                    .unwrap_or_else(|| "(unnamed)".to_owned()),
                mime_type: attachment.mime_type.clone(),
                size: postio_ui::format::human_size(attachment.size),
            })
            .collect(),
    }
}

/// The draft as the store takes it.
///
/// `base` is what the boundary last knew about this draft — its kind, its
/// ancestor, the `Message-ID` reserved for it — none of which the frontend
/// edits and all of which would be lost by rebuilding from the fields alone.
pub(crate) fn from_ffi(base: Draft, edited: &DraftFfi) -> Draft {
    let mut draft = base;
    draft.to = parse(&edited.to);
    draft.cc = parse(&edited.cc);
    draft.bcc = parse(&edited.bcc);
    draft.subject = edited.subject.clone();
    draft.body = body_of(edited);
    draft.rich = edited.rich;
    draft
}

/// The body a draft is stored with, from what the composer sent over.
///
/// # Rich is not "keep the HTML the editor had"
///
/// The surface hands over its DOM's `innerHTML`, which is a working copy and
/// never the record. It is narrowed to the [`Document`] dialect here, and the
/// document is what is rendered back out -- so the stored HTML is always
/// something `postio_body` can hold, whatever the editor or the paste
/// produced, and the round trip is idempotent rather than accumulating
/// whatever a web view felt like emitting.
///
/// # The plain alternative is derived, never trusted
///
/// `text` for a rich draft comes from the same document, flowed. It is not
/// taken from `edited.body`, because that field is what a *plain* composer
/// types into and a rich one has no reason to keep current -- and a frontend
/// that forgot would send an empty `text/plain` to everyone reading without
/// HTML. Deriving it means the two alternatives cannot disagree.
///
/// [`Document`]: postio_body::Document
fn body_of(edited: &DraftFfi) -> MessageBody {
    if !edited.rich {
        return MessageBody {
            text: Some(edited.body.clone()),
            // Kept, not cleared. The switch is on the document (#1271):
            // turning it off changes what will be *built*, and throwing the
            // marks away would make turning it back on a loss nobody warned
            // about.
            html: edited.body_html.clone(),
        };
    }

    let document = postio_body::parse(edited.body_html.as_deref().unwrap_or_default());
    let (text, html) = postio_body::render(&document);
    MessageBody {
        text: Some(text),
        html: Some(html),
    }
}

/// `Ada Norwood <ada@example.com>, bo@example.com`.
fn render(addresses: &[postio_model::EmailAddress]) -> String {
    addresses
        .iter()
        .map(|address| address.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// What a person typed into a recipient field.
fn parse(text: &str) -> Vec<postio_model::EmailAddress> {
    address::parse_list(text)
}

/// Why a change to a draft was refused.
///
/// An error rather than an `Option<String>` because these have a *result* to
/// carry when they work — the draft as it now stands — and a frontend that
/// got back `None` and had to re-read the draft itself would be a frontend
/// that could disagree with the store about what is attached.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ComposeError {
    /// It could not be done, and this is why in words for the person who
    /// asked.
    #[error("{message}")]
    Refused {
        /// What went wrong.
        message: String,
    },
}

/// What a paste became, and what it cost.
///
/// Both halves of the acceptance line in one answer: the markup narrowed to
/// the dialect, and the sentence saying what the dialect could not hold. The
/// sentence is `postio_body::Lost::summary`'s, so the two composers say the
/// same thing about the same paste.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PastedFfi {
    /// The paste as the dialect holds it, ready to put in the document.
    pub html: String,
    /// The same content as plain text, for the `text/plain` alternative and
    /// for a composer that is not in rich mode.
    pub text: String,
    /// What was lost, as one sentence, or `None` when nothing was.
    ///
    /// `None` rather than an empty string, and silence is the right answer:
    /// a composer that announced every paste would train people to ignore
    /// the one that mattered.
    pub dropped: Option<String>,
}
