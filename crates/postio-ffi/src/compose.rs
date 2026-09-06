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
    /// The body, as text. Rich composition keeps the same text and adds
    /// marks; see [`rich`](Self::rich).
    pub body: String,
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
        rich: draft.body.html.is_some(),
        in_reply_to: draft.in_reply_to.map(|id| id.get()),
        path,
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
    draft.body = MessageBody {
        text: Some(edited.body.clone()),
        // Rich composition is its own surface (#1271); until it exists the
        // HTML part is not invented here, and `rich` says what *will* be
        // sent rather than what has been typed.
        html: draft.body.html.clone(),
    };
    draft
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
