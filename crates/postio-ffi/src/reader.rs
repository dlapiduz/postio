//! The reader's document, and the inline parts it may reference.

/// Whether the reader may fetch images the message points at.
///
/// Crosses because it is the frontend's per-sender decision to make, and
/// **`Blocked` is the default everywhere**: `PRODUCT.md`'s "nothing leaves this
/// machine that the user did not ask for" starts here, at the tracking pixel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum RemoteImagesFfi {
    /// Images the message points at are not fetched.
    Blocked,
    /// The sender is allowed, so remote images load.
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

/// One part of a message, referenced from its body by `Content-ID`.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InlinePart {
    /// The bytes, already on this machine.
    pub bytes: Vec<u8>,
    /// What the part says it is, for the response's `Content-Type`.
    pub mime_type: String,
}

/// One standing permission to load remote images (#1156, Privacy pane).
///
/// A grant the user cannot see is one they cannot take back, and `PRODUCT.md`
/// promises images are blocked *until allowed per sender* — which only means
/// something if what has been allowed is reviewable.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct GrantFfi {
    /// The address or the domain, as the list stores it.
    pub subject: String,
    /// Whether this covers every address at a domain rather than one sender.
    ///
    /// Worth showing plainly: the two are very different amounts of trust,
    /// and a list that drew them alike would understate one of them.
    pub whole_domain: bool,
}

/// What the reader is holding back, and about whom.
///
/// Drawn as one row that never wraps (canvas screen 26): an icon, the count,
/// `Show`, and a `⋯` that opens the grants. It is **per message**, never per
/// pane — a conversation of eight messages can hold back pictures from three
/// different senders, and one notice above them all could not say whose.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReaderNoticeFfi {
    /// The sentence: `6 remote images blocked`.
    pub summary: String,
    /// The address the grant would be about.
    pub sender: String,
    /// Its domain, for the second grant — the one that covers a service
    /// which never sends twice from the same address.
    pub domain: String,
    /// Whether this sender is already allowed, in which case the reader has
    /// stopped asking.
    pub allowed: bool,
    /// How many remote images were held back, and how many of those were
    /// tracking pixels.
    ///
    /// The numbers as well as the sentence, because they answer a different
    /// question: `part_held_back_note` decides whether *Render once* is
    /// offered at all and takes counts, not wording. A frontend reading them
    /// back out of "6 remote images blocked" would be a second reader of a
    /// string written for people, and it would break the first time the
    /// sentence was reworded.
    pub remote_images: u32,
    /// How many of them were tracking pixels — a 1x1 whose only job is to
    /// report that the message was opened.
    pub trackers: u32,
}

/// An address with its middle elided: `notices_at_…@relay.example.net`.
///
/// The middle is what goes because both ends identify the sender — the local
/// part says which service and the domain says whose it is — and a grant is a
/// decision about exactly that. A tail-truncated address hides the half a
/// person judges by. `postio_ui::format::middle_truncate`'s wording, so the
/// two frontends elide the same address the same way.
#[uniffi::export]
pub fn middle_truncate(text: String, width: u32) -> String {
    postio_ui::format::middle_truncate(&text, width as usize)
}

/// Who an open message was addressed to, already rendered.
///
/// Read per open message rather than carried on every list row: a mailbox is
/// never loaded into memory (`PRODUCT.md` §18), and `To` and `Cc` are
/// questions asked about the message in front of you, not about forty rows at
/// once. Strings rather than addresses, and rendered by
/// `postio_ui::reader::header` rather than in a frontend, so both readers
/// abbreviate one recipient list the same way (#1150 is what the other answer
/// cost on a field whose whole job is to be read).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RecipientsFfi {
    /// The whole `To:` line, label included, or `None` when the message names
    /// no recipient the store kept — nothing, rather than a blank line.
    pub to: Option<String>,
    /// The `Cc` addresses, joined; `None` when there are none, which is what
    /// lets a header spend no space at all on the common case.
    pub cc: Option<String>,
    /// What the `Cc` disclosure is called while it is offered — `Cc (2)`.
    pub cc_label: Option<String>,
}

/// One verb the reading pane offers under a message.
///
/// Which four and in what order is
/// `postio_ui::reader::header::ReaderAction`'s call, so the two frontends
/// cannot end up offering different bars. **No key travels with it**: this
/// platform draws `⌘R`, not `e`, and the chord is
/// `Session::binding_for` — the verb list is shared, the way it is
/// spelled on a keyboard is not.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ReaderActionFfi {
    /// The registry command it runs — never a local implementation, or it
    /// would be a second way to archive that undo did not know about.
    pub command: String,
    /// What the button is labelled.
    pub title: String,
    /// Whether it gets the primary treatment. Exactly one does.
    pub primary: bool,
}

/// Where a page turn lands, given where the reader is now.
///
/// The reading pane's only scroll primitive is a same-document fragment
/// navigation to the markers the shared document lays down — JavaScript is
/// off in that view on purpose (ADR 0003), so there is no scroll-by-amount
/// call to make. This is which marker to jump to, and it crosses rather than
/// being done in Swift so the two frontends page identically.
#[uniffi::export]
pub fn reader_page_after(current: u32, forward: bool) -> u32 {
    postio_ui::reader::document::page_after(current, forward)
}

/// The fragment for marker `page`: `pos-7`.
///
/// One spelling, shared with the anchors themselves, or a page key silently
/// does nothing and nothing anywhere says why.
#[uniffi::export]
pub fn reader_page_fragment(page: u32) -> String {
    postio_ui::reader::document::page_fragment(page)
}

/// The invisible anchors a page turn jumps between.
///
/// Crosses only so a test can build a document with them in it and prove the
/// key actually moves the page — the application's documents already carry
/// them, because `postio_ui::reader::document` puts them there.
#[uniffi::export]
pub fn reader_scroll_markers() -> String {
    postio_ui::reader::document::scroll_markers()
}

/// Everything the pane asks about one open message, in one answer (#1589).
///
/// Opening a message used to be four calls — notice, caveat, unsubscribe
/// offer, recipients — and between them they loaded and decompressed the
/// body three times and ran the sanitizer twice, once purely to count
/// blocked images. This is those four answers off one row read, one body
/// load and one render.
///
/// Every field is optional because every fact is: the common personal
/// message has no pictures held back, no list to leave and nothing wrong
/// with its body, and an absent fact must be `None` rather than an empty
/// something — a blank banner is still a banner.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MessageFactsFfi {
    /// What the reader held back, or `None` when nothing was.
    pub notice: Option<ReaderNoticeFfi>,
    /// What to say about a body that lost something in decoding.
    pub caveat: Option<String>,
    /// The list this message offers to leave, or `None` for a person.
    pub offer: Option<crate::UnsubscribeOfferFfi>,
    /// Who it was addressed to, or `None` for a message that is gone.
    pub recipients: Option<RecipientsFfi>,
}
