/// Whether the reader may fetch images: `Blocked` is the default everywhere, since nothing leaves
/// the machine that the user did not ask for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteImagesFfi {
    Blocked,
    Allowed,
}

impl From<RemoteImagesFfi> for postio_body::RemoteImages {
    fn from(remote: RemoteImagesFfi) -> Self {
        match remote {
            RemoteImagesFfi::Blocked => postio_body::RemoteImages::Blocked,
            RemoteImagesFfi::Allowed => postio_body::RemoteImages::Allowed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InlinePart {
    pub bytes: Vec<u8>,
    pub mime_type: String,
}

/// One standing permission to load remote images (#1156, Privacy pane): a grant the user cannot see
/// is one they cannot take back.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GrantFfi {
    pub subject: String,
    /// Whether this covers every address at a domain rather than one sender: a very different
    /// amount of trust.
    pub whole_domain: bool,
}

/// What the reader is holding back, and about whom. Per message, never per pane: a conversation can
/// hold back pictures from several senders.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReaderNoticeFfi {
    /// The sentence: `6 remote images blocked`.
    pub summary: String,
    pub sender: String,
    /// Its domain, for the second grant: it covers a service that never sends twice from the same
    /// address.
    pub domain: String,
    /// Whether this sender is already allowed, in which case the reader has
    /// stopped asking.
    pub allowed: bool,
    /// How many remote images were held back. Counts as well as the sentence: `part_held_back_note`
    /// decides whether
    /// *Render once* is offered from counts, not wording.
    pub remote_images: u32,
    /// How many of them were tracking pixels.
    pub trackers: u32,
}

/// An address with its middle elided: `notices_at_…@relay.example.net`. Both ends identify the
/// sender, so the middle goes
/// (`postio_ui::format::middle_truncate`).
#[uniffi::export]
pub fn middle_truncate(text: String, width: u32) -> String {
    postio_ui::format::middle_truncate(&text, width as usize)
}

/// Who an open message was addressed to, rendered by `postio_ui::reader::header` so both readers
/// abbreviate alike. Read per
/// open message, not carried on every list row.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecipientsFfi {
    /// The whole `To:` line, label included, or `None` when the message names no recipient the
    /// store kept.
    pub to: Option<String>,
    /// The `Cc` addresses, joined; `None` when there are none.
    pub cc: Option<String>,
    /// What the `Cc` disclosure is called while it is offered — `Cc (2)`.
    pub cc_label: Option<String>,
}

/// One verb the reading pane offers under a message. Which, and in what order, is
/// `postio_ui::reader::header::ReaderAction`'s
/// call. No key travels with it: the chord is `Session::binding_for`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReaderActionFfi {
    /// The registry command it runs, never a local implementation: undo would not know about it.
    pub command: String,
    pub title: String,
    /// Whether it gets the primary treatment. Exactly one does.
    pub primary: bool,
}

/// One verb of a conversation's header bar. Reply, Reply all and Forward go to the latest message
/// and Archive takes the whole
/// thread, so `description` says so before either is pressed: the tooltip and accessible name
/// (`ReaderAction::describe`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ConversationActionFfi {
    pub command: String,
    pub title: String,
    pub primary: bool,
    /// What it will do, in words -- "Reply to the latest message".
    pub description: String,
    pub whole_conversation: bool,
}

/// Where a page turn lands, given where the reader is now. JavaScript is off in that view (ADR
/// 0003), so the only scroll
/// primitive is a fragment navigation to the document's markers.
#[uniffi::export]
pub fn reader_page_after(current: u32, forward: bool) -> u32 {
    postio_ui::reader::document::page_after(current, forward)
}

/// The fragment for marker `page`: `pos-7`. One spelling, shared with the anchors, or a page key
/// silently does nothing.
#[uniffi::export]
pub fn reader_page_fragment(page: u32) -> String {
    postio_ui::reader::document::page_fragment(page)
}

/// The invisible anchors a page turn jumps between; crosses so a test can build a document with
/// them.
#[uniffi::export]
pub fn reader_scroll_markers() -> String {
    postio_ui::reader::document::scroll_markers()
}

/// The row's own facts about one open message (#1589): everything a pane can know without the body.
/// Every field is optional:
/// an absent fact is `None`, not an empty something.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageFactsFfi {
    /// The list this message offers to leave, or `None` for a person.
    pub offer: Option<crate::UnsubscribeOfferFfi>,
    /// Who it was addressed to, or `None` for a message that is gone.
    pub recipients: Option<RecipientsFfi>,
}

/// The reader's whole answer for one message: the document, and the two facts that fall out of
/// rendering it (#1589): one load, one render.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReaderDocumentFfi {
    /// The whole document, ready to hand a web view. Never empty: a missing
    /// body is a state plate, which is also a document.
    pub html: String,
    /// What this render held back, or `None`. Only a blocked render can answer: an allowed render
    /// would answer "nothing".
    pub notice: Option<ReaderNoticeFfi>,
    /// What to say about a body that lost something in decoding, or `None`.
    /// Render-independent: it rides the load, not the render.
    pub caveat: Option<String>,
}
