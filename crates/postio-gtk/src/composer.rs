//! The composer: canvas 2a, taking over the reading pane.
//!
//! # Why it is not a window
//!
//! The original brief implied a separate compose window; canvas 2a is explicit
//! that it is not, and docs/PRODUCT.md §10 records that as settled. The composer
//! replaces the *reader* and nothing else, so the message list stays exactly
//! where it was — same scroll offset, same selection, same widths. Half of
//! writing a reply is looking at what you are replying to and at what else is
//! waiting, and a window that covers the list takes both away.
//!
//! # It is a mode, so it says so
//!
//! Modes are expensive: the user has to know which one they are in and how to
//! get out. This one is announced four ways, none of which is a dialog:
//!
//! * The reading pane is *entirely* the composer — there is no half state.
//! * Its heading names the composition (`Compose`, `Reply`, `Forward`).
//! * The sidebar and the list dim while it is open, the canvas' own signal
//!   that the keyboard belongs somewhere else. High contrast is exempt; see
//!   `shell.css`.
//! * The footer says `Esc keeps the draft` — the exit, and what it costs.
//!
//! # Escape never discards
//!
//! `Esc` closes the composer and the draft stays in it, so reopening compose
//! puts the user back in front of what they were writing. Discarding is a
//! separate verb (`ctrl+d`, [`CommandId::DiscardDraft`]), it is the only one
//! the registry marks destructive, and it asks first when there is anything to
//! lose. [`closing`] is the rule, and it is unit-tested rather than trusted.
//!
//! One composition at a time: there is one reading pane, so there is one
//! composer. Opening compose while a started draft is retained reopens *that*
//! draft rather than replacing it, and says so on the status line. Several
//! drafts at once wants the Drafts mailbox to be real first (`postio-own`).
//!
//! # What this bead does not build
//!
//! The composer edits a [`Draft`] and hands it back; it does not construct a
//! message, queue it, or save it.
//!
//! * MIME construction is `postio-es9`, reached from a [`connect_send`] handler
//!   with the [`Draft`] this widget produces.
//! * Sending through the operation queue is `postio-pzy`, and reply seeding
//!   and quoting is `postio-p8q` — which builds the [`Draft`] that
//!   [`Composer::open`] takes.
//! * Autosave (`postio-own`), attachments (`postio-tws`) and recipient
//!   completion (`postio-agd`) each end at a seam this widget owns —
//!   [`connect_save`], [`connect_attach`] and
//!   [`connect_recipient_suggestions`] — and stop there. What debounces a
//!   save, draws an attachment row, or shows a completion popover lives here;
//!   writing any of it to disk is `postio-storage`'s job, reached through
//!   whoever wires the seam.
//!
//! Until a handler is connected the composer says so instead of pretending:
//! [`CommandId::Send`] with nothing listening leaves the draft in place and
//! names the missing piece on the status line, because a composer that empties
//! itself into a seam that is not connected yet has silently lost the mail.
//! [`CommandId::AttachFile`] and a drop with no [`connect_attach`] handler do
//! the same.
//!
//! [`connect_send`]: Composer::connect_send
//! [`connect_save`]: Composer::connect_save
//! [`connect_attach`]: Composer::connect_attach
//! [`connect_recipient_suggestions`]: Composer::connect_recipient_suggestions

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{DateTime, Datelike, Duration, Local, Utc};
use gtk::{gdk, gio, glib};
use postio_body::Placement;
use postio_core::{CommandId, Context};
use postio_model::address::{current_entry, format_list, parse_list};
use postio_model::{
    Account, AccountId, Attachment, Draft, DraftKind, EmailAddress, Identity, IdentityId, Message,
    MessageBody, Signature, SignatureId,
};
use postio_model::{reply, signature};

use crate::shell::Pane;
use crate::window::Window;

/// The field the keyboard lands in when the composer opens.
///
/// Two rules, from the bead: a reply focuses the body, because the recipients
/// and the subject are already decided; new mail focuses `To`, because nothing
/// is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    /// The `To` field.
    To,
    /// The body.
    Body,
}

/// Where the keyboard goes when a composition of this kind opens.
///
/// Reply and reply-all come from `reply_draft` with the original sender or
/// recipients already filled into `to`, so the keyboard can go straight to
/// the body. Forward has no recipient at all -- there is nobody to forward
/// to until the user picks one -- so it belongs with `New`, not with the
/// replies (#690).
pub fn first_field(kind: DraftKind) -> Field {
    match kind {
        DraftKind::New | DraftKind::Forward => Field::To,
        DraftKind::Reply | DraftKind::ReplyAll => Field::Body,
    }
}

/// What the composer's heading says it is.
///
/// The vocabulary is the app's: **Compose**, never "New" or "Write".
pub fn heading(kind: DraftKind) -> &'static str {
    match kind {
        DraftKind::New => "Compose",
        DraftKind::Reply => "Reply",
        DraftKind::ReplyAll => "Reply all",
        DraftKind::Forward => "Forward",
    }
}

/// What closing the composer does with the draft in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Closing {
    /// Keep it: reopening compose comes back to it.
    Keep,
    /// Nothing was written, so there is nothing to keep.
    Drop,
}

/// Whether closing the composer has anything to keep.
///
/// The acceptance criterion "`Esc` never silently discards content" is this
/// function: anything the user typed — a recipient, a subject, a word of body —
/// makes the draft worth keeping. Only a composition that is still exactly as
/// it opened is dropped, and dropping *that* discards nothing.
///
/// Neither whitespace nor the signature counts as content. A body holding
/// only what the composer put there would make every abandoned composer
/// permanent, which is how a "we kept your draft" message stops meaning
/// anything.
pub fn closing(draft: &Draft) -> Closing {
    let body = draft.body.text.as_deref().unwrap_or_default();
    // The signature is the composer's own doing, not something the user
    // wrote, so a body holding nothing else is still an untouched composer.
    let written = signature::split(body).0;
    if draft.has_recipients() || !draft.subject.trim().is_empty() || !written.trim().is_empty() {
        Closing::Keep
    } else {
        Closing::Drop
    }
}

/// What to say about recipients that will not survive contact with a server.
///
/// A warning, never a refusal: the text stays in the field, and the count is
/// the only thing that changes. Refusing the keystroke would lose what was
/// typed and explain nothing.
pub fn recipient_warning(draft: &Draft) -> Option<String> {
    let wrong = draft
        .all_recipients()
        .filter(|address| !address.is_plausible())
        .count();
    match wrong {
        0 => None,
        1 => Some("1 address does not look like an address".to_owned()),
        many => Some(format!("{many} addresses do not look like addresses")),
    }
}

/// Whether `host` belongs to a different organisation than `sender_domain`.
///
/// Exact match or a subdomain of it counts as the same: a company's own
/// `click.shop.example.org` linking out from `shop.example.org` is not what
/// [`quoted_tracking_domains`] exists to flag, only a host with no
/// relationship to the sender's domain at all.
fn differs_from_sender(host: &str, sender_domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let sender_domain = sender_domain.to_ascii_lowercase();
    host != sender_domain && !host.ends_with(&format!(".{sender_domain}"))
}

/// What the composer says when a reply's quoted text links to one or more
/// domains other than the message being replied to came from.
///
/// `None` for nothing to say — this is issue #116's whole shape: no link is
/// ever stripped or rewritten by anything upstream (a link is not a load,
/// `postio-body/tests/outgoing.rs::a_quoted_link_keeps_its_href_because_a_link_is_not_a_load`),
/// this only decides whether to say something about one.
fn tracking_link_notice(domains: &[String]) -> Option<String> {
    match domains {
        [] => None,
        [one] => Some(format!(
            "The quoted text links to {one}, which differs from the sender's own domain."
        )),
        many => Some(format!(
            "The quoted text links to {} domains that differ from the sender's own: {}.",
            many.len(),
            many.join(", ")
        )),
    }
}

/// Hosts a reply's quoted text would link to that differ from `source`'s own
/// sender.
///
/// A reply reaches people the original message did not, and a tracking
/// redirect's query string usually encodes the *first* recipient's id — so a
/// reply-all's other readers would click through under that id, not their
/// own, without anyone having decided to send their identifier anywhere.
/// Purely informational (issue #116's maintainer verdict): `postio-body`
/// still represents exactly what the message said, and nothing here changes
/// what ends up in the draft.
///
/// Scans exactly what the quote will contain, by construction: the same
/// [`source_document`] the reply is built from (#340). A message whose only
/// links live in a part that is not quoted — plain text carries no `<a>` —
/// produces nothing to warn about.
fn quoted_tracking_domains(source: &Message) -> Vec<String> {
    let Some(sender_domain) = source.primary_from().and_then(|from| from.domain()) else {
        return Vec::new();
    };
    source_document(source)
        .link_hosts()
        .into_iter()
        .filter(|host| differs_from_sender(host, sender_domain))
        .collect()
}

/// What the status line says before anything has happened to the draft.
/// What the body field announces itself as. One constant because the scroll
/// region around it is a separate tab stop and must say the same thing.
const BODY_NAME: &str = "Message body";

const UNSAVED: &str = "draft is in the composer only";

/// What the status line says when opening compose came back to a kept draft.
const RETAINED: &str = "still holding the draft Esc kept";

/// What the status line says when [`CommandId::Send`] has nowhere to go.
///
/// Reachable only with no account configured at all: the composition root
/// connects [`Composer::connect_send`] whenever it has one. Before #423 it
/// connected it never, so this was what sending did — always, on every
/// account — and the wording was plausible enough to read as a setting
/// somebody had missed rather than as a seam nobody had wired.
const NO_SEND_PATH: &str = "not sent — no outgoing account is connected yet";

/// What the status line says when Reply/Reply All/Forward is pressed while
/// the composer already has a draft open.
///
/// #426: `open` already refuses to replace a *retained* draft (the
/// `Closing::Keep` branch, `RETAINED`'s own case) — typed prose with nowhere
/// else to go is not something a second keystroke should be able to lose.
/// The same refusal has to hold for a draft still in progress, and it has to
/// say so: the bug this fixes was not the refusal, which is right, but that
/// refusing looked identical to the key doing nothing at all.
const REPLY_BLOCKED: &str = "not opened — finish or close the current draft first";

/// What the status line says when `ctrl+Return` is pressed on a draft that is
/// addressed to nobody.
const NO_RECIPIENTS: &str = "not sent — add a recipient first";

/// What the status line says when `ctrl+Return` is pressed on a draft that has
/// already been handed over: it is the queue's now, not the composer's.
const ALREADY_QUEUED: &str = "not sent again — this draft is already on its way";

/// What the status line says when a file was chosen or dropped but nothing
/// is listening on [`Composer::connect_attach`] to turn it into an attachment.
const NO_ATTACH_PATH: &str = "not attached — no attachment handler is connected yet";

/// What the status line says when an image was pasted but nothing is
/// listening on [`Composer::connect_inline_image`] to store its bytes.
/// What the picker's first entry says: sign the way this identity signs.
const SIGNATURE_FROM_IDENTITY: &str = "Identity signature";

const NO_INLINE_PATH: &str = "not inserted — no inline-image handler is connected yet";

/// Above this, an attachment's row calls out its size.
///
/// The bead asks for a *configurable* threshold; that means reading
/// `postio-config`, which this crate does not touch. A fixed default earns
/// its keep until that wiring exists rather than silently warning never.
const LARGE_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// How long an edit waits before [`Composer::save`] runs again on its own.
///
/// Long enough that a burst of keystrokes coalesces into one save rather than
/// one per character — the acceptance criterion is that autosave never
/// blocks typing, and firing on every keystroke would eventually compete with
/// it even if each save were fast. Short enough that a crash loses at most
/// this much of a sentence.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);

/// The class `shell.css` dims the sidebar and the list under.
///
/// Canvas 2a's own signal that the keyboard is in the composer: the list is
/// still there, still scrolled where it was, and visibly not what is being
/// typed into.
pub const COMPOSING_CLASS: &str = "composing";

/// How big the detached composer opens, in logical pixels.
///
/// Deliberately narrower than the main window: it is one message being
/// written, not a mail client, and a pop-out that opens the size of the
/// window it came out of reads as a second application. Tall enough that the
/// header rows and a paragraph of body are both visible without scrolling,
/// which is the state most replies are finished in.
const DETACHED_SIZE: (i32, i32) = (620, 560);

/// What to call with the draft when the user sends or saves.
type DraftHandler = Box<dyn Fn(&Draft)>;

/// What to call with the draft when the user schedules it to send later.
///
/// Carries the chosen time because, unlike an ordinary send, there is no
/// other way for the handler to learn it — the picker is entirely inside the
/// composer and nothing downstream can ask it again once the draft is handed
/// off and the fields are cleared.
type SendLaterHandler = Box<dyn Fn(&Draft, DateTime<Utc>)>;

/// What to call with the draft when the user asks it to be saved.
///
/// `&mut`, unlike [`DraftHandler`]: the storage-backed handler assigns a
/// [`DraftId`](postio_model::DraftId) the first time it persists a draft, and
/// has nowhere else to put it back — the composer never constructs its own
/// repository row. [`Composer::save`] writes the id back onto its own draft
/// afterward, which is what makes every save after the first an update to the
/// same row instead of a fresh insert.
type SaveHandler = Box<dyn Fn(&mut Draft)>;

/// What to call when the composer closes, with what became of the draft.
type ClosedHandler = Box<dyn Fn(Closing)>;

/// What to call when the composer takes over the reading pane.
type OpenedHandler = Box<dyn Fn()>;

/// One row of recipient completion: a single address, or a named group that
/// expands to every one of its members the moment it is accepted.
///
/// ADR 0007 Q3: there is no group address to insert instead — a draft's
/// recipients have to be what the user can see, which is what keeps `Bcc`
/// honest and stops a draft's recipients from silently changing if someone
/// edits the group's membership after it was picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecipientCandidate {
    /// One address, exactly as accepting it always worked.
    Contact(EmailAddress),
    /// A named group. `members` is the membership at the moment this
    /// candidate was offered — accepting it inserts all of them as
    /// individual addresses, never a group reference.
    Group {
        /// Display name, for the completion row.
        name: String,
        /// Every member's address, in the order they are inserted.
        members: Vec<EmailAddress>,
    },
}

/// Answers "what does `prefix` complete to" for recipient completion —
/// contacts, previous correspondents and contact groups, ranked by frequency
/// and recency, are [`Composer::connect_recipient_suggestions`]'s job to
/// supply; the composer only shows what it is given, in the order it is
/// given.
type RecipientSuggestions = Box<dyn Fn(&str) -> Vec<RecipientCandidate>>;

/// Answers "what message is `e`/`E`/`f` about" — the composer has no notion
/// of a reading pane or a selection of its own. `None` means there is nothing
/// to reply to right now (nothing open, or nothing to send as), in which case
/// the keystroke does nothing rather than opening a broken composer.
type ReplySourceProvider = Box<dyn Fn() -> Option<(Message, Account)>>;

/// Answers "what should a brand-new draft sign with, before the identity's
/// own?" (#394) — the composer has no notion of which mailbox the sidebar has
/// selected, so whatever tracks that resolves the precedence and hands back
/// only the answer. `None` means neither the mailbox nor the account has an
/// opinion, and the picker stays on the identity's own signature.
type SignatureDefaultProvider = Box<dyn Fn() -> Option<SignatureId>>;

/// What [`Composer::connect_attach`] hands its result to, exactly once:
/// `Some` with the finished attachment, `None` to reject the file (unreadable,
/// say). Calling this is what actually adds the row — synchronously, for a
/// handler that already has the answer, or from a spawned task's own
/// callback, for one that had to go read the file first. `pub` because it
/// appears in `connect_attach`'s public signature, not because anything
/// outside this module constructs one.
pub type AttachReady = Box<dyn FnOnce(Option<Attachment>)>;

/// What [`Composer::connect_inline_image`] hands its result to, exactly
/// once: `Some` with the finished attachment — carrying a `Content-ID` and
/// `Disposition::Inline` — or `None` to reject the paste. `pub` because it
/// appears in the public signature, like [`AttachReady`].
pub type InlineImageReady = Box<dyn FnOnce(Option<Attachment>)>;

/// Turns pasted image bytes into an inline attachment: writes them to the
/// blob store, mints a `Content-ID`, and calls [`InlineImageReady`] whenever
/// it is ready — the same non-blocking contract as [`AttachHandler`].
type InlineImageHandler = Box<dyn Fn(Vec<u8>, String, InlineImageReady)>;

/// The slot `build()` fills with the draft-aware `postio-cid:` lookup; the
/// editor's blob source redirects through it for the composer's whole life.
type BlobLookup = Rc<RefCell<Option<Box<dyn Fn(&str) -> Option<(Vec<u8>, String)>>>>>;

/// Reads an attachment's bytes back, for the editing surface's
/// `postio-cid:` requests. Synchronous and local, like
/// [`crate::reader::scheme::BlobSource`], because that is what a scheme
/// handler can await.
type AttachmentBytes = Box<dyn Fn(&Attachment) -> Option<Vec<u8>>>;

/// Turns a chosen or dropped file into attachment metadata, calling
/// [`AttachReady`] with the result whenever it is ready. A handler that
/// blocks here to read a large file blocks the composer along with it; slow
/// work belongs on a thread or task the handler spawns itself, calling back
/// once it finishes — `add_file` never waits on this closure to return.
type AttachHandler = Box<dyn Fn(std::path::PathBuf, AttachReady)>;

/// Which of [`postio_model::reply`]'s three functions a command asks for.
///
/// [`CommandId::Reply`], [`CommandId::ReplyAll`] and [`CommandId::Forward`]
/// are the only commands this maps, and only when the composer is not
/// already open — replying to a reply in progress is not a thing.
fn reply_draft(id: CommandId, source: &Message, account: &Account) -> Option<Draft> {
    match id {
        CommandId::Reply => Some(reply::reply(source, account, quoted_body(source, false))),
        CommandId::ReplyAll => Some(reply::reply_all(
            source,
            account,
            quoted_body(source, false),
        )),
        CommandId::Forward => Some(reply::forward(source, account, quoted_body(source, true))),
        _ => None,
    }
}

/// The body a reply or forward starts from, built from the parsed document —
/// ADR 0003 Q3's inversion, done in the crate that has both halves.
///
/// Rich, in both renderings: the HTML half is what the editor opens
/// (`document_of` prefers it), and the text half is the same document's
/// `to_text`, whose `> ` convention keeps the plain form every mail client
/// expects. Building both from one [`postio_body::Document`] is the
/// security property (hardening requirement 6): a script or a tracking
/// pixel in the source has no representation in the document, so neither
/// rendering can carry one.
fn quoted_body(source: &Message, forward: bool) -> MessageBody {
    let document = source_document(source);
    let rich = if forward {
        postio_body::forwarded(&document, &reply::forward_header(source))
    } else {
        postio_body::quoted_reply(&document, &reply::attribution(source))
    };
    let (text, html) = postio_body::render(&rich);
    MessageBody {
        text: Some(text),
        html: Some(html),
    }
}

/// The document `source`'s body means — the markup the reader showed when
/// there is markup, the plain text otherwise.
///
/// The plain-text fallback goes through [`Document::from_flowed_text`]
/// rather than [`Document::from_text`] exactly when `source` itself
/// declared `format=flowed` (#456): unwrapping unconditionally would take
/// an ordinary sender's own short lines as soft breaks and join them, and
/// never unwrapping would show a `format=flowed` sender's wrapped sentence
/// — including this app's own past sends — as line breaks nobody typed.
///
/// [`Document::from_flowed_text`]: postio_body::Document::from_flowed_text
/// [`Document::from_text`]: postio_body::Document::from_text
fn source_document(source: &Message) -> postio_body::Document {
    match (&source.body.html, &source.body.text) {
        (Some(html), _) => postio_body::parse(html),
        (None, Some(text)) if source.text_is_flowed => {
            postio_body::Document::from_flowed_text(text)
        }
        (None, Some(text)) => postio_body::Document::from_text(text),
        (None, None) => postio_body::Document::new(),
    }
}

mod imp {
    use super::*;

    pub struct Composer {
        pub heading: gtk::Label,
        pub status: gtk::Label,
        pub to: gtk::Entry,
        pub cc: gtk::Entry,
        pub bcc: gtk::Entry,
        pub subject: gtk::Entry,
        pub cc_row: gtk::Box,
        pub bcc_row: gtk::Box,
        pub more: gtk::Button,
        /// The pointer's way to the same command the keyboard and the palette
        /// reach. Its label flips, because one control that says which way it
        /// goes beats two that are each wrong half the time.
        pub detach: gtk::Button,
        /// The window the composition is in when it is not in the pane.
        ///
        /// `Some` is the whole of "detached" — there is no flag to disagree
        /// with it, because a flag and a window can drift apart and a window
        /// that is not there cannot.
        pub detached: RefCell<Option<adw::Window>>,
        pub identity_row: gtk::Box,
        /// The account's named signatures, and the picker over them (#12).
        /// Entry 0 is the identity's own; the rest are the account's set, so
        /// a draft can sign differently without changing who it is from.
        pub signatures: RefCell<Vec<Signature>>,
        pub signature: gtk::DropDown,
        pub identity: gtk::DropDown,
        pub identity_only: gtk::Label,
        /// The editing surface: ADR 0003's WebView, adopted for every
        /// draft (ADR 0004 Q6 as amended, #347). The `Editor` owns the
        /// document, the edit history and the typing-run coalescing — the
        /// composer holds a view over its record, exactly as ADR 0004
        /// always demanded of the surface.
        pub body: crate::editor::Editor,
        /// The formatting toolbar's five toggles, in registry order: bold,
        /// italic, bullet list, numbered list, quote block. Toggles because
        /// each reflects the caret — active when the selection already sits
        /// inside what the button would apply.
        pub format_toggles: [gtk::ToggleButton; 5],
        /// The link button: a plain button, because a link is a dialog to
        /// fill in, not a state the caret can be in or out of.
        pub link_button: gtk::Button,
        pub send: gtk::Button,
        /// Opens the [`CommandId::ScheduleSend`] picker beside `send`. Its
        /// popover content is rebuilt fresh from [`schedule_presets`] every
        /// time it opens — see `set_create_popup_func` in `build_actions` —
        /// because the presets are relative to whatever moment the picker
        /// opens, not to when the composer itself was built.
        pub schedule_send: gtk::MenuButton,
        pub save: gtk::Button,
        /// "Esc keeps the draft", trailing the action row. The least
        /// essential of the four (#692): it ellipsizes under a narrow
        /// allocation rather than the row overflowing the window with a
        /// bare edge-clip, or a button's own label losing a word.
        pub escape: gtk::Label,
        pub warning: gtk::Label,
        /// Issue #116: "this reply quotes a link to a domain other than the
        /// sender's own" — purely informational, next to `warning` but a
        /// separate label, since the two can be true at once and are about
        /// unrelated things.
        pub tracking_notice: gtk::Label,
        /// The attachment list's own row, hidden entirely while there is
        /// nothing to show — a bare "no attachments" line is clutter the Cc
        /// row already taught this composer not to add.
        pub attachments_box: gtk::Box,
        pub attachments_list: gtk::ListBox,
        /// Everything about the draft that is not in a field: its id, account,
        /// kind, attachments, and what it is a reply to.
        pub draft: RefCell<Draft>,
        pub identities: RefCell<Vec<Identity>>,
        pub sent: RefCell<Vec<DraftHandler>>,
        pub sent_later: RefCell<Vec<SendLaterHandler>>,
        pub saved: RefCell<Vec<SaveHandler>>,
        pub changed: RefCell<Vec<DraftHandler>>,
        pub closed: RefCell<Vec<ClosedHandler>>,
        /// Called when a fresh `open()` actually takes the pane — not on the
        /// no-op path where the composer was already open. What the header's
        /// `Compose` button listens to, alongside `closed`, to say `Composing`
        /// only while that is true.
        pub opened: RefCell<Vec<OpenedHandler>>,
        /// Where `e`/`E`/`f` get the message and account to reply to. One
        /// slot, last registration wins: there is exactly one reading pane.
        pub reply_source: RefCell<Option<ReplySourceProvider>>,
        /// What a brand-new draft should sign with, before the identity's own
        /// (#394) — read at the moment `c` starts one, since which mailbox is
        /// selected changes far more often than the account itself does.
        pub signature_default: RefCell<Option<SignatureDefaultProvider>>,
        /// Where recipient completion gets its candidates. One slot: the
        /// same search serves `To`, `Cc` and `Bcc` alike.
        pub recipient_suggestions: RefCell<Option<RecipientSuggestions>>,
        /// Where the signature sits on a reply and on a forward (#12), from
        /// `[compose]`. New mail has no quote, so placement cannot mean
        /// anything there.
        pub signature_placement: Cell<(Placement, Placement)>,
        /// Where a chosen or dropped file becomes attachment metadata. One
        /// slot: the same handler serves the file chooser and drag-and-drop.
        pub attach: RefCell<Option<AttachHandler>>,
        /// Where pasted image bytes become an inline attachment (#341).
        pub inline_image: RefCell<Option<InlineImageHandler>>,
        /// Where an inline attachment's bytes come back from for display.
        pub attachment_bytes: RefCell<Option<AttachmentBytes>>,
        /// What the body's `postio-cid:` requests resolve through — filled
        /// by `build()` with a draft-aware lookup, held here because the
        /// editor is constructed before the composer object exists.
        pub blob_lookup: BlobLookup,
        /// The window the composer took a pane from, once mounted.
        pub window: glib::WeakRef<Window>,
        /// The context and pane to put back when the composer closes.
        pub restore: Cell<Option<(Context, Pane)>>,
        /// Set while `open` is filling the fields, so the widgets' own
        /// `changed` signals do not report the fill as the user typing.
        pub filling: Cell<bool>,
        /// The pending debounced autosave, if an edit is waiting out the
        /// quiet period before [`Composer::save`] runs again.
        pub autosave_source: Cell<Option<glib::SourceId>>,
        /// Recipient completion for `To`. Kept here (rather than relying on
        /// the entry's signal connections to be the only thing keeping it
        /// alive) so tests can reach it; see `test_accept_recipient_suggestion`.
        pub(crate) to_completion: RefCell<Option<Rc<Completion>>>,
    }

    impl Default for Composer {
        fn default() -> Self {
            let row = || gtk::Box::new(gtk::Orientation::Horizontal, 14);
            let blob_lookup: BlobLookup = Rc::new(RefCell::new(None));
            Self {
                heading: gtk::Label::new(None),
                status: gtk::Label::new(Some(UNSAVED)),
                to: gtk::Entry::new(),
                cc: gtk::Entry::new(),
                bcc: gtk::Entry::new(),
                subject: gtk::Entry::new(),
                cc_row: row(),
                bcc_row: row(),
                more: gtk::Button::new(),
                detach: gtk::Button::new(),
                detached: RefCell::new(None),
                identity_row: row(),
                signatures: RefCell::new(Vec::new()),
                signature: gtk::DropDown::from_strings(&[]),
                identity: gtk::DropDown::from_strings(&[]),
                identity_only: gtk::Label::new(None),
                body: {
                    let lookup = blob_lookup.clone();
                    crate::editor::Editor::new(std::rc::Rc::new(move |content_id: &str| {
                        lookup
                            .borrow()
                            .as_ref()
                            .and_then(|lookup| lookup(content_id))
                    }))
                },
                format_toggles: std::array::from_fn(|_| gtk::ToggleButton::new()),
                link_button: gtk::Button::new(),
                send: gtk::Button::new(),
                schedule_send: gtk::MenuButton::new(),
                save: gtk::Button::new(),
                escape: gtk::Label::new(None),
                warning: gtk::Label::new(None),
                tracking_notice: gtk::Label::new(None),
                attachments_box: gtk::Box::new(gtk::Orientation::Vertical, 6),
                attachments_list: gtk::ListBox::new(),
                draft: RefCell::new(Draft::new(AccountId::UNASSIGNED)),
                identities: RefCell::new(Vec::new()),
                sent: RefCell::new(Vec::new()),
                sent_later: RefCell::new(Vec::new()),
                saved: RefCell::new(Vec::new()),
                changed: RefCell::new(Vec::new()),
                closed: RefCell::new(Vec::new()),
                opened: RefCell::new(Vec::new()),
                reply_source: RefCell::new(None),
                signature_default: RefCell::new(None),
                recipient_suggestions: RefCell::new(None),
                signature_placement: Cell::new((Placement::AboveQuote, Placement::AboveQuote)),
                attach: RefCell::new(None),
                inline_image: RefCell::new(None),
                attachment_bytes: RefCell::new(None),
                blob_lookup,
                window: glib::WeakRef::new(),
                restore: Cell::new(None),
                filling: Cell::new(false),
                autosave_source: Cell::new(None),
                to_completion: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Composer {
        const NAME: &'static str = "PostioComposer";
        type Type = super::Composer;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Composer {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Composer {}
    impl BinImpl for Composer {}
}

glib::wrapper! {
    /// The composer, filling the reading pane.
    pub struct Composer(ObjectSubclass<imp::Composer>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Composer {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Composer {
    /// A composer holding an empty draft, not yet on screen.
    pub fn new() -> Self {
        Self::default()
    }

    // -- Opening and closing -------------------------------------------------

    /// Puts `draft` in the composer and takes over the reading pane.
    ///
    /// Instant: `set_visible` on widgets that are already built, no transition,
    /// nothing rebuilt. The list is not touched at all, which is what keeps its
    /// scroll offset and its selection where the user left them.
    ///
    /// A draft already in the composer with something in it wins over `draft`:
    /// see the module documentation for why one pane means one composition.
    pub fn open(&self, draft: Draft) {
        // Already composing: `c` a second time is a no-op that puts the
        // keyboard back, never a reset of what is being typed. Detached, the
        // keyboard is in another window, so put *that* in front instead —
        // there is one composition either way, and asking for it again means
        // "show me it", not "start another".
        if self.is_open() {
            if let Some(host) = self.detached_window() {
                host.present();
            }
            self.focus_first();
            return;
        }

        if closing(&self.imp().draft.borrow()) == Closing::Keep {
            self.set_status(RETAINED);
        } else {
            self.fill(draft);
            self.set_status(UNSAVED);
        }

        self.take_pane();
        self.set_visible(true);
        self.focus_first();

        for handler in self.imp().opened.borrow().iter() {
            handler();
        }
    }

    /// Put *this* draft in the composer, replacing whatever it was holding.
    ///
    /// [`open`](Self::open) deliberately refuses to replace a retained draft —
    /// `c` a second time means "show me the draft", never "start another",
    /// which is the one-composition-at-a-time rule. Resuming is a different
    /// request: a draft the user named, picked out of the Drafts folder. It
    /// has to replace, and it may, because a retained draft is autosaved and
    /// — since #166 — is itself a row in that folder. Swapping to another
    /// draft and back loses nothing.
    ///
    /// Whatever was pending is flushed first. The autosave is debounced, so a
    /// draft swapped away from a moment after an edit has that edit sitting in
    /// a timer, and the timer would otherwise fire against the draft that
    /// replaced it — writing one draft's words onto another's row.
    ///
    /// Asking for the draft already in the composer is a no-op that puts the
    /// keyboard back, exactly as [`open`](Self::open) is.
    pub fn resume(&self, draft: Draft) {
        if self.is_open() && self.imp().draft.borrow().id == draft.id {
            if let Some(host) = self.detached_window() {
                host.present();
            }
            self.focus_first();
            return;
        }
        // Before `fill`, which cancels the timer: a pending edit belongs to
        // the draft on its way out.
        if self.is_open() {
            self.save();
        }
        self.fill(draft);
        self.set_status(UNSAVED);
        self.take_pane();
        self.set_visible(true);
        self.focus_first();

        for handler in self.imp().opened.borrow().iter() {
            handler();
        }
    }

    /// Whether the composer is on screen.
    pub fn is_open(&self) -> bool {
        self.is_visible()
    }

    /// Which field has the keyboard, when one of them does.
    ///
    /// The focus rules are an acceptance criterion — a reply starts in the
    /// body, new mail starts in `To` — so they are worth being able to assert
    /// on rather than to look at.
    pub fn focused_field(&self) -> Option<Field> {
        let window = self.root().and_downcast::<gtk::Window>()?;
        let focus = gtk::prelude::GtkWindowExt::focus(&window)?;
        let imp = self.imp();
        // An entry hands the keyboard to the `GtkText` inside it, so the
        // focused widget is a descendant of the field rather than the field.
        if holds(&focus, imp.to.upcast_ref()) {
            Some(Field::To)
        } else if holds(&focus, imp.body.widget().upcast_ref()) {
            Some(Field::Body)
        } else {
            None
        }
    }

    /// Closes the composer, keeping whatever was written.
    ///
    /// Returns what happened to the draft, which is what the caller reports:
    /// [`Closing::Keep`] means it is still here, [`Closing::Drop`] that there
    /// was nothing in it. Never destroys typed content — that is
    /// [`Composer::discard`], and it asks first.
    pub fn close(&self) -> Closing {
        let draft = self.draft();
        let outcome = closing(&draft);
        if outcome == Closing::Drop {
            self.fill(Draft::new(draft.account_id));
        } else {
            *self.imp().draft.borrow_mut() = draft;
        }

        self.shut(outcome);
        outcome
    }

    /// Takes the composer off screen and tells everyone what became of it.
    fn shut(&self, outcome: Closing) {
        // A debounced save still waiting out its quiet period must not be
        // left ticking against a composer nobody can see: flush it now, so
        // an edit made just before closing is on the same footing as one made
        // a second earlier that the timer already caught.
        self.flush_autosave();
        // Closing a detached composition closes the window it is in — and it
        // comes back to the pane on the way, so the next `c` finds the
        // composer where it always is rather than in a window that is gone.
        // Reparented before hiding, never after: a widget with no parent has
        // no root, and `focused_field` and the pane restore below both ask
        // the root what it is doing.
        self.reclaim();
        self.set_visible(false);
        self.release_pane();
        for handler in self.imp().closed.borrow().iter() {
            handler(outcome);
        }
    }

    /// Throws the draft away and closes. What `ctrl+d` does once confirmed.
    pub fn discard(&self) {
        let account = self.imp().draft.borrow().account_id;
        self.fill(Draft::new(account));
        self.close();
    }

    /// Asks before discarding, when there is anything to lose.
    ///
    /// The registry marks `discard_draft` [`Recovery::Confirm`][r] — the one
    /// composer verb that does, because typed prose exists nowhere else and
    /// the undo stack cannot bring it back. An empty draft is discarded
    /// without a question, since there is nothing to protect.
    ///
    /// [r]: postio_core::Recovery
    pub fn request_discard(&self) {
        if closing(&self.draft()) == Closing::Drop {
            self.discard();
            return;
        }

        let dialog = adw::AlertDialog::new(
            Some("Discard this draft?"),
            Some("What you have written will be gone. Esc closes the composer and keeps it."),
        );
        dialog.add_responses(&[("keep", "Keep"), ("discard", "Discard")]);
        dialog.set_response_appearance("discard", adw::ResponseAppearance::Destructive);
        dialog.set_default_response(Some("keep"));
        dialog.set_close_response("keep");
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |_, response| {
                    if response == "discard" {
                        composer.discard();
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    // -- The draft ------------------------------------------------------------

    /// The draft as it stands, fields included.
    pub fn draft(&self) -> Draft {
        let imp = self.imp();
        let mut draft = imp.draft.borrow().clone();
        draft.to = parse_list(&imp.to.text());
        draft.cc = parse_list(&imp.cc.text());
        draft.bcc = parse_list(&imp.bcc.text());
        draft.subject = imp.subject.text().to_string();
        draft.body = self.body();
        draft
    }

    /// The body as the wire wants it, rendered from the document.
    ///
    /// `text` always; `html` only when the document carries structure that
    /// `to_text` would lose. A message somebody typed as plain text is sent
    /// as plain text: a `multipart/alternative` whose HTML half says exactly
    /// what its text half says is bytes nobody asked for, a second thing to
    /// get wrong, and the thing mailing lists ask people not to send.
    ///
    /// Note `html` is *regenerated*, never carried over. Before this it was
    /// passed through from whatever the draft was opened with, so editing the
    /// text of a reply left an HTML half describing the text before the edit.
    ///
    /// `text` is [`Document::to_flowed_text`], not `to_text` — RFC 3676
    /// `format=flowed` (#333), soft-wrapped at 72 columns. This is the one
    /// place that matters: the `MessageBody` built here is what a draft is
    /// filed as *and* what actually gets sent, so wrapping happens exactly
    /// once, before either.
    ///
    /// [`source_document`] undoes exactly this wrapping when it reads a
    /// message back — including one this composer sent — through
    /// `Message::text_is_flowed`, the fact `#333` could not check yet and
    /// `#456` added.
    fn body(&self) -> MessageBody {
        let document = self.document();
        // `html`'s half of `render` costs a real `to_html` and a `harden`
        // pass; keeping it behind `is_plain_text()`'s check, as before,
        // means a plain-text draft never pays for HTML it is about to
        // throw away.
        let text = document.to_flowed_text();
        MessageBody {
            text: (!text.is_empty()).then_some(text),
            html: (!document.is_plain_text()).then(|| postio_body::render(&document).1),
        }
    }

    /// An absorbed edit: the [`Editor`](crate::editor::Editor) has already
    /// recorded it on the document's history and coalesced the typing run;
    /// what is left is the composer's own reactions to the body moving.
    fn body_edited(&self) {
        // Filling the fields is not an edit. The Editor's own load path
        // never reports one, but programmatic fills flow through here too.
        if self.imp().filling.get() {
            return;
        }
        self.refresh();
    }

    /// Step the body back one typing run. Swallowed at the floor rather
    /// than propagated: Ctrl+Z at the start of the history must not fall
    /// through to whatever else in the window would answer it.
    fn undo_edit(&self) -> glib::Propagation {
        self.imp().body.undo();
        self.refresh();
        glib::Propagation::Stop
    }

    /// Step it forward again.
    fn redo_edit(&self) -> glib::Propagation {
        self.imp().body.redo();
        self.refresh();
        glib::Propagation::Stop
    }

    /// The body as it stands: the Editor's record. The surface is a copy of
    /// this, never the other way round — ADR 0004, unchanged by the surface
    /// swap that retired the `GtkTextView` (#347).
    pub fn document(&self) -> postio_body::Document {
        self.imp().body.document()
    }

    /// What the composer will send from, once identities are set.
    pub fn identity(&self) -> Option<Identity> {
        let selected = self.imp().identity.selected() as usize;
        self.imp().identities.borrow().get(selected).cloned()
    }

    /// The addresses this account can send from.
    ///
    /// One identity is not a choice, so it is drawn as a line of text rather
    /// than as a picker with nothing else in it.
    ///
    /// The account's default is selected, and its signature goes into the body
    /// straight away — the composer opens showing what will be sent, not a
    /// body that grows a signature at some later moment.
    pub fn set_identities(&self, identities: Vec<Identity>) {
        let imp = self.imp();
        let names: Vec<String> = identities.iter().map(identity_label).collect();
        imp.identity.set_model(Some(&gtk::StringList::new(
            &names.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        imp.identity_only.set_text(
            names
                .first()
                .map(String::as_str)
                .unwrap_or("no identity configured"),
        );
        imp.identity.set_visible(identities.len() > 1);
        imp.identity_only.set_visible(identities.len() <= 1);

        let default = identities.iter().position(|identity| identity.is_default);
        *imp.identities.borrow_mut() = identities;
        imp.identity.set_selected(default.unwrap_or(0) as u32);
        self.apply_identity();
    }

    /// Sends this draft as the identity with `id`, if the account has it.
    ///
    /// The override is the *draft's*: it is recorded on the draft, so closing
    /// the composer and coming back to it sends from the same address, and a
    /// later reply picks its own identity again rather than inheriting this
    /// one.
    pub fn select_identity(&self, id: IdentityId) -> bool {
        let Some(index) = self
            .imp()
            .identities
            .borrow()
            .iter()
            .position(|identity| identity.id == id)
        else {
            return false;
        };
        self.imp().identity.set_selected(index as u32);
        self.apply_identity();
        true
    }

    /// Offers `signatures` in the picker, alongside the identity's own.
    ///
    /// Hidden entirely when the account has none: a picker with one entry is
    /// a control that can only ever say what is already true.
    pub fn set_signatures(&self, signatures: Vec<Signature>) {
        let imp = self.imp();
        let mut labels = vec![SIGNATURE_FROM_IDENTITY.to_owned()];
        labels.extend(signatures.iter().map(|signature| signature.name.clone()));
        imp.signature.set_model(Some(&gtk::StringList::new(
            &labels.iter().map(String::as_str).collect::<Vec<_>>(),
        )));
        imp.signature.set_visible(!signatures.is_empty());
        *imp.signatures.borrow_mut() = signatures;
        imp.signature.set_selected(0);
        self.apply_identity();
    }

    /// Selects the named signature with `id` in the picker, if the account
    /// has it. `false` when it does not — the caller falls back to the
    /// identity's own rather than leaving a stale selection in place.
    fn select_signature(&self, id: SignatureId) -> bool {
        let Some(index) = self
            .imp()
            .signatures
            .borrow()
            .iter()
            .position(|signature| signature.id == id)
        else {
            return false;
        };
        // Entry 0 is always the identity's own; the account's signatures
        // follow it, so the picker index is one past the account's own.
        self.imp().signature.set_selected((index + 1) as u32);
        true
    }

    /// Opens a fresh, unaddressed draft — what `c` starts when nothing is
    /// already open. Resolves what it signs with through
    /// [`connect_signature_default`](Self::connect_signature_default) rather
    /// than leaving whatever a previous compose left the picker on: a mailbox
    /// override applies here, an account default here, and "neither has an
    /// opinion" resets the picker to the identity's own rather than an older
    /// draft's choice (#394).
    fn open_new_draft(&self) {
        self.open(Draft::new(self.account()));
        let resolved = self
            .imp()
            .signature_default
            .borrow()
            .as_ref()
            .and_then(|provider| provider());
        let selected = resolved.is_some_and(|id| self.select_signature(id));
        if !selected {
            self.imp().signature.set_selected(0);
        }
    }

    /// The signature this draft signs with: whichever the picker names, or
    /// the identity's own when it names none.
    fn chosen_signature(&self, identity: &Identity) -> Option<Signature> {
        let imp = self.imp();
        // An empty picker reports `INVALID_LIST_POSITION`, not zero — so this
        // cannot be a subtraction on a raw index: entry 0 *and* "no entries at
        // all" both mean the identity's own signature.
        let chosen = match imp.signature.selected() {
            gtk::INVALID_LIST_POSITION => None,
            selected => (selected as usize).checked_sub(1),
        };
        match chosen {
            Some(index) => imp.signatures.borrow().get(index).cloned(),
            None => identity.signature.clone(),
        }
    }

    /// Sets where a signature goes on a reply and on a forward, from
    /// `[compose]`. Live: the next identity or signature change uses it.
    pub fn set_signature_placement(&self, on_reply: Placement, on_forward: Placement) {
        self.imp().signature_placement.set((on_reply, on_forward));
    }

    /// The placement this draft's kind asks for.
    fn signature_placement(&self) -> Placement {
        let (on_reply, on_forward) = self.imp().signature_placement.get();
        match self.imp().draft.borrow().kind {
            DraftKind::Reply | DraftKind::ReplyAll => on_reply,
            DraftKind::Forward => on_forward,
            // Nothing is quoted, so both placements put it in the same place.
            DraftKind::New => Placement::BelowQuote,
        }
    }

    /// Puts the selected identity, and its signature, into the draft.
    ///
    /// Idempotent, because [`Draft::use_identity`] replaces the signature
    /// block rather than appending one: switching identity mid-compose swaps
    /// signatures, and re-running this over an unchanged draft changes
    /// nothing.
    fn apply_identity(&self) {
        let Some(identity) = self.identity() else {
            return;
        };
        let imp = self.imp();
        let mut draft = self.draft();
        draft.use_identity(&identity);
        imp.draft.borrow_mut().identity_id = draft.identity_id;

        // At the block level for every draft, never through text: flattening
        // a rich quote to `> ` lines to swap a signature would be the
        // identity dropdown quietly destroying the reply (#340). The plain
        // form is still exactly right — `to_text` renders the separator and
        // the signature the way the wire wants them — so one path serves
        // both, and placement (#12) means the same thing in each.
        let current = imp.body.document();
        let signature = self.chosen_signature(&identity);
        let wanted =
            postio_body::apply_signature(&current, signature.as_ref(), self.signature_placement());
        if current == wanted {
            return;
        }

        // The written part above the signature is untouched; the caret goes
        // to the start of the body, which is where it lives in a reply
        // anyway. Preserving the exact offset across an Editor reload needs
        // a script round trip the identity switch has not yet earned — the
        // GtkTextView used to keep it precisely, and a caret assertion is
        // the test to bring back if that ever matters again.
        let was_filling = imp.filling.replace(true);
        imp.body.load(wanted);
        imp.body.place_caret_start();
        imp.filling.set(was_filling);
        self.refresh();
    }

    /// The status line under the heading.
    pub fn status(&self) -> String {
        self.imp().status.text().to_string()
    }

    /// Says what just happened to the draft — saved, queued, or not yet.
    ///
    /// Set by whoever actually did it. The composer never claims a draft was
    /// saved on its own behalf; it has nowhere to save it to.
    pub fn set_status(&self, status: &str) {
        self.imp().status.set_text(status);
    }

    // -- The verbs ------------------------------------------------------------

    /// Hands the draft to the send handlers and closes. What `ctrl+Enter` does.
    ///
    /// With nothing connected the draft stays exactly where it is and the
    /// status line names what is missing, rather than the composer emptying
    /// itself into a seam that does not exist yet.
    ///
    /// # Why the key checks what the button already checks
    ///
    /// [`refresh`](Self::refresh) greys the Send button out on
    /// [`Draft::is_sendable`], so the pointer has never been able to send an
    /// unaddressed or already-queued draft. `ctrl+Return` reaches this
    /// directly and had no such guard, which cost nothing while the seam was
    /// unwired — every send was a no-op. Now that it queues, the two ways of
    /// asking for the same thing have to agree, and the two disagreements
    /// both destroy something:
    ///
    /// * **No recipients.** The composer would clear and close, and the queued
    ///   operation would drain as impossible — the words gone, and no message
    ///   sent.
    /// * **Already queued.** A draft resumed from the Drafts folder between
    ///   the enqueue and the drain is still listed there, so it can be opened
    ///   and sent a second time. That is a second `Operation::Send` against
    ///   one draft, which is the recipient receiving the message twice.
    pub fn send(&self) {
        if self.imp().sent.borrow().is_empty() {
            self.set_status(NO_SEND_PATH);
            return;
        }
        let draft = self.draft();
        if !draft.is_sendable() {
            self.set_status(if draft.has_recipients() {
                ALREADY_QUEUED
            } else {
                NO_RECIPIENTS
            });
            return;
        }
        for handler in self.imp().sent.borrow().iter() {
            handler(&draft);
        }
        let account = draft.account_id;
        self.fill(Draft::new(account));
        self.shut(Closing::Drop);
    }

    /// Hands the draft to the send-later handlers with `when`, and closes.
    /// What choosing a time from the [`CommandId::ScheduleSend`] picker does.
    ///
    /// The same sendability checks [`send`](Self::send) makes, for the same
    /// reasons: an unaddressed or already-queued draft must not be handed to
    /// a handler here any more than to an immediate send.
    pub fn send_later(&self, when: DateTime<Utc>) {
        if self.imp().sent_later.borrow().is_empty() {
            self.set_status(NO_SEND_PATH);
            return;
        }
        let draft = self.draft();
        if !draft.is_sendable() {
            self.set_status(if draft.has_recipients() {
                ALREADY_QUEUED
            } else {
                NO_RECIPIENTS
            });
            return;
        }
        for handler in self.imp().sent_later.borrow().iter() {
            handler(&draft, when);
        }
        let account = draft.account_id;
        self.fill(Draft::new(account));
        self.shut(Closing::Drop);
    }

    /// Hands the draft to the save handlers. What `ctrl+s` does.
    ///
    /// A handler may assign the draft an id (its first persisted save); that
    /// id is written back onto this composer's own draft afterward, so the
    /// next save updates the same row rather than inserting a second one.
    /// Nothing else a handler touches is kept — the fields are the widgets'
    /// to own, not a save handler's.
    pub fn save(&self) {
        let mut draft = self.draft();
        for handler in self.imp().saved.borrow().iter() {
            handler(&mut draft);
        }
        self.imp().draft.borrow_mut().id = draft.id;
    }

    /// Called with the draft when the user sends it.
    pub fn connect_send(&self, handler: impl Fn(&Draft) + 'static) {
        self.imp().sent.borrow_mut().push(Box::new(handler));
    }

    /// Called with the draft and the chosen time when the user schedules it
    /// to send later.
    pub fn connect_send_later(&self, handler: impl Fn(&Draft, DateTime<Utc>) + 'static) {
        self.imp().sent_later.borrow_mut().push(Box::new(handler));
    }

    /// Called with the draft when the user asks for it to be saved.
    ///
    /// `&mut Draft`: see [`SaveHandler`] for why a save handler is allowed to
    /// write back the id it persisted under.
    pub fn connect_save(&self, handler: impl Fn(&mut Draft) + 'static) {
        self.imp().saved.borrow_mut().push(Box::new(handler));
    }

    /// Called with the draft on every edit — the seam autosave hangs off.
    pub fn connect_changed(&self, handler: impl Fn(&Draft) + 'static) {
        self.imp().changed.borrow_mut().push(Box::new(handler));
    }

    /// Called when the composer closes, with what became of the draft.
    pub fn connect_closed(&self, handler: impl Fn(Closing) + 'static) {
        self.imp().closed.borrow_mut().push(Box::new(handler));
    }

    /// Called when the composer takes over the reading pane — not when `c`
    /// (or the header button) finds it already open and just moves the
    /// keyboard back to it.
    pub fn connect_opened(&self, handler: impl Fn() + 'static) {
        self.imp().opened.borrow_mut().push(Box::new(handler));
    }

    /// Registers where `e`/`E`/`f` find the message and account to reply to.
    ///
    /// The composer holds no reading-pane state of its own; whatever tracks
    /// the message currently on screen connects this once, at mount time.
    pub fn connect_reply_source(
        &self,
        provider: impl Fn() -> Option<(Message, Account)> + 'static,
    ) {
        *self.imp().reply_source.borrow_mut() = Some(Box::new(provider));
    }

    /// Registers what a brand-new draft should sign with, before the
    /// identity's own (#394) — called once, at the moment `c` starts one, so
    /// it always answers for whichever mailbox is selected right then.
    ///
    /// `None` from the composer's own reads — nothing registered here, or the
    /// provider itself answering `None` — leaves the picker on the identity's
    /// own signature, exactly as it already was without this seam.
    pub fn connect_signature_default(&self, provider: impl Fn() -> Option<SignatureId> + 'static) {
        *self.imp().signature_default.borrow_mut() = Some(Box::new(provider));
    }

    /// Registers what completes a recipient prefix — contacts, previous
    /// correspondents and contact groups, ranked and searched however the
    /// caller sees fit. The composer only shows what comes back, in that
    /// order, and never touches the network itself: purely local completion
    /// is the whole point.
    pub fn connect_recipient_suggestions(
        &self,
        provider: impl Fn(&str) -> Vec<RecipientCandidate> + 'static,
    ) {
        *self.imp().recipient_suggestions.borrow_mut() = Some(Box::new(provider));
    }

    /// Registers what turns a chosen or dropped file into attachment
    /// metadata — and, however long it takes, into the blob store. The
    /// composer only draws the row; storing the bytes is the caller's job.
    ///
    /// `handler` is given the path and a callback to invoke with the result;
    /// it may call the callback immediately or hand it to a background task
    /// and return right away. Either way `add_file` does not block on it.
    pub fn connect_attach(&self, handler: impl Fn(std::path::PathBuf, AttachReady) + 'static) {
        *self.imp().attach.borrow_mut() = Some(Box::new(handler));
    }

    /// Wires the seam pasted image bytes go out through to become an inline
    /// attachment — blob write and `Content-ID` minting live above this
    /// crate. Same non-blocking contract as [`connect_attach`](Self::connect_attach).
    pub fn connect_inline_image(
        &self,
        handler: impl Fn(Vec<u8>, String, InlineImageReady) + 'static,
    ) {
        *self.imp().inline_image.borrow_mut() = Some(Box::new(handler));
    }

    /// Wires where an attachment's bytes come back from, for the editing
    /// surface to display inline images — this draft's pasted ones and a
    /// resumed draft's alike. Synchronous and local, like a blob-store read.
    pub fn connect_attachment_bytes(
        &self,
        reader: impl Fn(&Attachment) -> Option<Vec<u8>> + 'static,
    ) {
        *self.imp().attachment_bytes.borrow_mut() = Some(Box::new(reader));
    }

    // -- Detaching ------------------------------------------------------------

    /// Whether the composition is in a window of its own rather than the pane.
    pub fn is_detached(&self) -> bool {
        self.imp().detached.borrow().is_some()
    }

    /// The window the composition is in, while it is in one.
    ///
    /// Public because "is it modal, and does it belong to the main window"
    /// are acceptance criteria rather than implementation, and because the
    /// application root needs somewhere to hang a title on.
    pub fn detached_window(&self) -> Option<adw::Window> {
        self.imp().detached.borrow().clone()
    }

    /// Moves the composition between the reading pane and its own window.
    ///
    /// What `ctrl+shift+o`, the palette entry and the pop-out button all do.
    /// A no-op when nothing is being composed: there is no empty composer to
    /// detach, and offering one would be a second way to start writing.
    pub fn toggle_detached(&self) {
        if !self.is_open() {
            return;
        }
        if self.is_detached() {
            self.attach();
        } else {
            self.detach();
        }
    }

    /// Pops the composition out into a window, and gives the pane back.
    ///
    /// The same widget, reparented — which is the entire reason nothing is
    /// lost. Every field, the identity override, the undo history and the
    /// cursor are in widgets that are never rebuilt, so "detaching keeps
    /// them" is a property of doing it this way rather than a list of things
    /// to remember to copy. A second composer built from the draft would have
    /// to copy each of them, and would be wrong about one of them eventually.
    pub fn detach(&self) {
        if self.is_detached() || !self.is_open() {
            return;
        }
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
        let field = self.focused_field();

        // The main window goes all the way back first: the message returns to
        // the reading pane and the keyboard context leaves `Composer`, so
        // from here on the two windows are independent and the main one is as
        // usable as it was before `c`.
        self.release_pane();

        let host = adw::Window::builder()
            .title(heading(self.imp().draft.borrow().kind))
            .transient_for(&window)
            // Not modal, and this is the criterion rather than a default:
            // the point of detaching is to read something else while you
            // write, and a modal is precisely the thing that forbids it.
            .modal(false)
            .default_width(DETACHED_SIZE.0)
            .default_height(DETACHED_SIZE.1)
            .build();

        // A real header bar, not a bare content window. `AdwWindow` draws no
        // titlebar of its own, so `set_content(composer)` would give a window
        // with no title, no close button and nothing to drag — and the pane
        // it came from has a header, so the pop-out looking like a stray
        // rectangle would read as a bug rather than a window. This is also
        // what CLAUDE.md means by keeping real Adwaita chrome so Postio reads
        // as a GNOME application.
        let layout = adw::ToolbarView::new();
        layout.add_top_bar(&adw::HeaderBar::new());
        window.shell().reader().remove(self);
        layout.set_content(Some(self));
        host.set_content(Some(&layout));
        self.set_visible(true);

        // Its keys are the same keys. The controller is a forwarder to the
        // main window's resolver so that `[keys]` reaches both containers and
        // there is only ever one keymap to keep in step.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| composer.handle_key(key, state)
        ));
        host.add_controller(keys);

        // The window's own close button means what `Esc` means: keep the
        // draft. Discarding stays explicit and still asks first.
        host.connect_close_request(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_| {
                composer.close();
                // `close` has already reparented and destroyed the window;
                // letting the default handler run as well would tear down a
                // window that is no longer there.
                glib::Propagation::Stop
            }
        ));

        self.imp().detached.replace(Some(host.clone()));
        sync_detach_button(&self.imp().detach, true);
        host.present();
        self.restore_focus(field);
    }

    /// Puts the composition back in the reading pane, window and all.
    pub fn attach(&self) {
        let field = self.focused_field();
        if !self.reclaim() {
            return;
        }
        self.take_pane();
        self.set_visible(true);
        self.restore_focus(field);
    }

    /// Puts the keyboard back in the field it was in before a reparent.
    ///
    /// Unparenting a widget drops the focus, so this is the one part of the
    /// composition that moving it really does lose — the text, the cursor and
    /// the history all ride along in widgets that were never rebuilt. Falls
    /// back to the field a fresh composition would start in, which is only
    /// reached when the keyboard was somewhere else entirely.
    fn restore_focus(&self, field: Option<Field>) {
        let imp = self.imp();
        match field {
            Some(Field::Body) => {
                imp.body.widget().grab_focus();
            }
            Some(Field::To) => {
                imp.to.grab_focus();
            }
            None => self.focus_first(),
        }
    }

    /// Reparents the composer back into the reading pane and disposes of the
    /// window, changing nothing else. `false` when it was not detached.
    ///
    /// Split out because closing a detached composer has to do this part and
    /// must *not* do the rest: [`attach`](Self::attach) takes the pane over
    /// again, which is the opposite of what closing wants.
    fn reclaim(&self) -> bool {
        let Some(host) = self.imp().detached.take() else {
            return false;
        };
        // Out of the toolbar view, not off the window: the composer is the
        // layout's content, and unparenting it from anywhere else would leave
        // GTK holding a child that is no longer there.
        if let Some(layout) = host.content().and_downcast::<adw::ToolbarView>() {
            layout.set_content(None::<&gtk::Widget>);
        }
        host.set_content(None::<&gtk::Widget>);
        if let Some(window) = self.imp().window.upgrade() {
            window.shell().reader().append(self);
        }
        // `destroy`, not `close`: `close` emits `close-request`, and this is
        // reached from inside that handler.
        host.destroy();
        sync_detach_button(&self.imp().detach, false);
        true
    }

    /// One key press from the detached window, resolved against the main
    /// window's keymap.
    ///
    /// Public for the same reason [`Window::handle_key`] is: it is the whole
    /// keyboard path in one call, and GTK4 gives no supported way to
    /// synthesize a GDK event for a test to press instead.
    ///
    /// [`Window::handle_key`]: crate::window::Window::handle_key
    pub fn handle_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        let Some(window) = self.imp().window.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let Some(host) = self.detached_window() else {
            return glib::Propagation::Proceed;
        };
        window.handle_key_in(key, state, &host, Context::Composer)
    }

    // -- Mounting -------------------------------------------------------------

    /// Puts the composer in `window`'s reading pane and wires the keyboard.
    ///
    /// After this, `c` opens the composer, `Esc` closes it keeping the draft,
    /// `ctrl+Enter` sends, `ctrl+s` saves and `ctrl+d` asks before discarding —
    /// all through the command registry, so the palette and the cheat sheet
    /// say the same thing the keys do. The header's Compose button reaches the
    /// same place through the `win.compose` action.
    pub fn mount(&self, window: &Window) {
        let imp = self.imp();
        imp.window.set(Some(window));

        let reader = window.shell().reader();
        self.set_vexpand(true);
        reader.append(self);
        // The pane's arbiter owns this widget's visibility from here on
        // (#502): hidden until the composer claims the pane.
        window
            .shell()
            .register_reader_occupant(crate::shell::ReaderOccupant::Composer, self.upcast_ref());

        let action = gio::SimpleAction::new("compose", None);
        action.connect_activate(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_, _| {
                // The header button reaches this action whether or not the
                // composer is open; the keyboard's `c` reaches `dispatch`
                // instead and stays a pure open (`open` is already a no-op
                // once composing, so `e`/`c` mid-reply cannot clobber it).
                // Only the button doubles as the close it now visibly is.
                if let Some(host) = composer.detached_window() {
                    // Detached, the button is not the close it is in the
                    // pane: the composition is in another window and the one
                    // useful thing to do is put it in front.
                    host.present();
                } else if composer.is_open() {
                    composer.close();
                } else {
                    composer.open_new_draft();
                }
            }
        ));
        window.add_action(&action);

        window.connect_command(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |id| composer.dispatch(id)
        ));

        if let Some(button) = window.compose_button() {
            sync_compose_button(&button, false);
            self.connect_opened({
                let button = button.clone();
                move || sync_compose_button(&button, true)
            });
            self.connect_closed(move |_outcome| sync_compose_button(&button, false));
        }
    }

    /// Acts on the commands the composer owns, and ignores the rest.
    fn dispatch(&self, id: CommandId) {
        match id {
            CommandId::Compose => self.open_new_draft(),
            CommandId::Send if self.is_open() => self.send(),
            CommandId::ScheduleSend if self.is_open() => self.imp().schedule_send.popup(),
            CommandId::SaveDraft if self.is_open() => self.save(),
            CommandId::DiscardDraft if self.is_open() => self.request_discard(),
            CommandId::AttachFile if self.is_open() => self.open_file_chooser(),
            CommandId::DetachComposer if self.is_open() => self.toggle_detached(),
            CommandId::Back if self.is_open() => {
                self.close();
            }
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward if !self.is_open() => {
                self.open_reply(id);
            }
            // #426: replacing an in-progress draft with a fresh reply would
            // be the one composer verb that loses typed prose with no
            // confirmation at all -- `request_discard` asks first for
            // exactly this reason. Falling through to silence was the bug;
            // the fix keeps the refusal and gives it a status line, the way
            // `send()` explains a missing send path instead of doing
            // nothing.
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward => {
                self.set_status(REPLY_BLOCKED);
            }
            CommandId::Bold if self.is_open() => {
                self.imp().body.format(crate::editor::Format::Bold);
            }
            CommandId::Italic if self.is_open() => {
                self.imp().body.format(crate::editor::Format::Italic);
            }
            CommandId::BulletList if self.is_open() => {
                self.imp().body.format(crate::editor::Format::BulletList);
            }
            CommandId::NumberedList if self.is_open() => {
                self.imp().body.format(crate::editor::Format::NumberedList);
            }
            CommandId::QuoteBlock if self.is_open() => {
                self.imp().body.format(crate::editor::Format::QuoteBlock);
            }
            CommandId::InsertLink if self.is_open() => self.request_link(),
            _ => {}
        }
    }

    /// Ask for an address, then link the selection to it.
    ///
    /// The one formatting command that needs an argument, so the one that
    /// opens a dialog — entry-first and Enter-to-confirm, since the hands
    /// are on the keyboard by construction.
    fn request_link(&self) {
        let dialog = adw::AlertDialog::new(
            Some("Link to…"),
            Some(
                "http, https or mailto. The selection becomes the link; with nothing selected, the address is inserted as its own text.",
            ),
        );
        let entry = gtk::Entry::builder()
            .placeholder_text("https://example.com/…")
            .activates_default(true)
            .build();
        entry.update_property(&[gtk::accessible::Property::Label("Link address")]);
        dialog.set_extra_child(Some(&entry));
        dialog.add_responses(&[("cancel", "Cancel"), ("link", "Link")]);
        dialog.set_response_appearance("link", adw::ResponseAppearance::Suggested);
        dialog.set_default_response(Some("link"));
        dialog.set_close_response("cancel");
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = composer)]
                self,
                #[weak]
                entry,
                move |_, response| {
                    if response != "link" {
                        return;
                    }
                    let href = entry.text().to_string();
                    if !composer.imp().body.create_link(&href) {
                        composer.set_status(
                            "That address was not linked: only http, https and mailto belong in mail.",
                        );
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// Opens a reply, reply-all or forward for whatever
    /// [`connect_reply_source`](Self::connect_reply_source) says is on
    /// screen. Silently does nothing without a source connected, or when the
    /// source has nothing to offer — `e` with no message open is not an
    /// error, it is nothing to reply to.
    fn open_reply(&self, id: CommandId) {
        let found = self
            .imp()
            .reply_source
            .borrow()
            .as_ref()
            .and_then(|provider| provider());
        let Some((source, account)) = found else {
            return;
        };
        // The picker has to speak for *this* account before `open` fills the
        // draft: `fill` resolves the draft's `identity_id` against whatever
        // `imp.identities` already holds, and that was last set for the
        // window's own default account -- never refreshed for a reply to a
        // message that arrived somewhere else (#189). Without this, a reply
        // to account B silently sent as account A's identity.
        self.set_identities(account.identities.clone());
        self.set_signatures(account.signatures.clone());
        if let Some(draft) = reply_draft(id, &source, &account) {
            // After, not before: `open` calls `fill`, which clears the
            // notice for every fresh composition — a reply's own domains
            // must win by running last, not be immediately wiped by it.
            self.open(draft);
            self.set_tracking_domains(&quoted_tracking_domains(&source));
        }
    }

    /// Show or hide issue #116's "this quotes a link to another domain"
    /// notice.
    fn set_tracking_domains(&self, domains: &[String]) {
        let notice = &self.imp().tracking_notice;
        match tracking_link_notice(domains) {
            Some(text) => {
                notice.set_text(&text);
                notice.set_visible(true);
            }
            None => notice.set_visible(false),
        }
    }

    fn account(&self) -> AccountId {
        self.imp().draft.borrow().account_id
    }

    /// Sets which account a fresh `Compose` starts from.
    ///
    /// For whoever assembles the application: the composer is built with no
    /// account at all ([`AccountId::UNASSIGNED`]), and nothing in
    /// `composer.rs` ever learns of one on its own. Meant to be called once,
    /// before the first composition — it writes straight onto the current
    /// draft, so calling it mid-compose would reassign whatever is already
    /// being written.
    pub fn set_account(&self, account_id: AccountId) {
        self.imp().draft.borrow_mut().account_id = account_id;
    }

    // -- Attachments ------------------------------------------------------

    /// Opens the platform file chooser for `ctrl+shift+a` and the "attach
    /// another" hint. `GtkFileDialog` goes through the XDG desktop portal on
    /// its own, which is what makes this work unmodified under Flatpak.
    fn open_file_chooser(&self) {
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
        let dialog = gtk::FileDialog::builder().title("Attach files").build();
        dialog.open_multiple(
            Some(&window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |result| {
                    let Ok(files) = result else {
                        return;
                    };
                    for i in 0..files.n_items() {
                        if let Some(file) = files.item(i).and_downcast::<gio::File>() {
                            composer.add_file(&file);
                        }
                    }
                }
            ),
        );
    }

    /// Wires dropping files anywhere on the composer to the same path as the
    /// file chooser.
    fn install_drop_target(&self) {
        let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        target.connect_drop(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(list) = value.get::<gdk::FileList>() else {
                    return false;
                };
                for file in list.files() {
                    composer.add_file(&file);
                }
                true
            }
        ));
        self.add_controller(target);
    }

    /// Turns a chosen or dropped file into an attachment via whatever
    /// [`connect_attach`](Self::connect_attach) is wired to, and says so on
    /// the status line when nothing is, rather than the file silently going
    /// nowhere.
    fn add_file(&self, file: &gio::File) {
        let Some(path) = file.path() else {
            return;
        };
        let handler = self.imp().attach.borrow();
        let Some(handler) = handler.as_ref() else {
            drop(handler);
            self.set_status(NO_ATTACH_PATH);
            return;
        };
        let composer = self.clone();
        handler(
            path,
            Box::new(move |attachment| match attachment {
                Some(attachment) => composer.add_attachment(attachment),
                None => composer.set_status(NO_ATTACH_PATH),
            }),
        );
    }

    /// Adds `attachment` to the draft and redraws the list. Counts as an
    /// edit: autosave and the `changed` handlers see it exactly like a typed
    /// word, which is what makes the debounce in `postio-own` cover it too.
    fn add_attachment(&self, attachment: Attachment) {
        self.imp().draft.borrow_mut().attachments.push(attachment);
        self.render_attachments();
        self.refresh();
    }

    /// Intercept `Ctrl+V` when the clipboard holds pixels; text falls
    /// through to WebKit's own paste. Returns whether the paste was taken.
    fn paste_image(&self) -> bool {
        let clipboard = self.imp().body.widget().clipboard();
        if !clipboard
            .formats()
            .contains_type(gdk::Texture::static_type())
        {
            return false;
        }
        let composer = self.clone();
        clipboard.read_texture_async(None::<&gio::Cancellable>, move |result| {
            let Ok(Some(texture)) = result else {
                return;
            };
            composer.add_inline_image(texture.save_to_png_bytes().to_vec(), "image/png");
        });
        true
    }

    /// The tail of a paste or drop of raw image bytes: out through
    /// [`connect_inline_image`](Self::connect_inline_image) to become an
    /// inline attachment, then into the draft and the body at the caret.
    fn add_inline_image(&self, bytes: Vec<u8>, mime_type: &str) {
        let handler = self.imp().inline_image.borrow();
        let Some(handler) = handler.as_ref() else {
            drop(handler);
            self.set_status(NO_INLINE_PATH);
            return;
        };
        let composer = self.clone();
        handler(
            bytes,
            mime_type.to_owned(),
            Box::new(move |attachment| match attachment {
                Some(attachment) => composer.add_inline_attachment(attachment),
                None => composer.set_status(NO_INLINE_PATH),
            }),
        );
    }

    /// Records the inline attachment and puts its image at the caret. The
    /// insertion crosses the bridge like any edit, so the document, undo and
    /// autosave all see it.
    fn add_inline_attachment(&self, attachment: Attachment) {
        let Some(content_id) = attachment
            .content_id
            .as_deref()
            .and_then(postio_body::ContentId::parse)
        else {
            self.set_status(NO_INLINE_PATH);
            return;
        };
        let alt = attachment
            .filename
            .clone()
            .unwrap_or_else(|| "image".to_owned());
        self.add_attachment(attachment);
        self.imp().body.insert_image(&content_id, &alt);
    }

    /// Removes the attachment at `index`. What removing a row before send
    /// does — the acceptance criterion "removing before send cleans up" is
    /// this and [`Composer::draft`] never seeing it again.
    fn remove_attachment(&self, index: usize) {
        let imp = self.imp();
        if index < imp.draft.borrow().attachments.len() {
            imp.draft.borrow_mut().attachments.remove(index);
        }
        self.render_attachments();
        self.refresh();
    }

    /// Rebuilds the attachment rows from the draft's own list, which stays
    /// the single source of truth — never the widgets, matching how `to`,
    /// `cc` and `bcc` work the other way around.
    fn render_attachments(&self) {
        let imp = self.imp();
        while let Some(row) = imp.attachments_list.row_at_index(0) {
            imp.attachments_list.remove(&row);
        }
        let attachments = imp.draft.borrow().attachments.clone();
        for (index, attachment) in attachments.iter().enumerate() {
            imp.attachments_list
                .append(&self.build_attachment_row(index, attachment));
        }
        imp.attachments_box.set_visible(!attachments.is_empty());
    }

    /// One row: name, size (flagging one past [`LARGE_ATTACHMENT_BYTES`]),
    /// and a button to remove it before send.
    fn build_attachment_row(&self, index: usize, attachment: &Attachment) -> gtk::Widget {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "Attachment: {}",
            attachment.display_name()
        ))]);

        let name = gtk::Label::new(Some(attachment.display_name()));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(pango::EllipsizeMode::Middle);

        let mut meta_text = format_size(attachment.size);
        if attachment.size >= LARGE_ATTACHMENT_BYTES {
            meta_text.push_str(" — large");
        }
        let meta = gtk::Label::new(Some(&meta_text));
        meta.add_css_class("postio-compose-label");
        meta.add_css_class("dim-label");

        let remove = gtk::Button::from_icon_name("edit-delete-symbolic");
        remove.add_css_class("flat");
        remove.update_property(&[gtk::accessible::Property::Label(&format!(
            "Remove {}",
            attachment.display_name()
        ))]);
        remove.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.remove_attachment(index)
        ));

        row.append(&name);
        row.append(&meta);
        row.append(&remove);
        row.upcast()
    }

    /// The attachment list's own row: a header naming the `ctrl+shift+a`
    /// hint per canvas 2a, and the rows themselves — hidden entirely until
    /// there is something to show.
    fn build_attachments(&self) -> gtk::Box {
        let imp = self.imp();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some("Attachments"));
        title.add_css_class("postio-compose-label");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        header.append(&title);
        header.append(&labelled("Attach another", "C-⇧-A"));

        imp.attachments_list
            .set_selection_mode(gtk::SelectionMode::None);
        imp.attachments_list
            .add_css_class("postio-compose-attachments");
        imp.attachments_list
            .set_accessible_role(gtk::AccessibleRole::ListBox);

        imp.attachments_box.append(&header);
        imp.attachments_box.append(&imp.attachments_list);
        imp.attachments_box.set_visible(false);
        imp.attachments_box.clone()
    }

    /// Hides whatever else is in the reading pane and remembers the way back.
    fn take_pane(&self) {
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
        let shell = window.shell();
        self.imp()
            .restore
            .set(Some((window.context(), shell.focused_pane())));

        // Through the pane's one owner (#502): the shell hides whichever
        // occupant had the pane and shows this composer. The old shape —
        // hide every sibling here, show every sibling on release — is what
        // put a search preview back under an open message.
        shell.set_composing(true);
        // In the one-pane mode the reader is not necessarily on screen, and a
        // composer the user cannot see is the worst possible mode.
        shell.set_focused_pane(Pane::Reader);
        shell.add_css_class(COMPOSING_CLASS);
        window.set_context(Context::Composer);
    }

    /// Gives the reading pane back to whatever is active now.
    fn release_pane(&self) {
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
        // Computed, not replayed: the shell shows what the current state
        // calls for — the search preview if search is up, the message the
        // pane was open on, or nothing. Showing every sibling here is the
        // #502 bug.
        window.shell().set_composing(false);
        window.shell().remove_css_class(COMPOSING_CLASS);
        if let Some((context, pane)) = self.imp().restore.take() {
            window.set_context(context);
            window.shell().set_focused_pane(pane);
        }

        // The keyboard is still in one of the composer's fields, which is
        // about to be a *hidden* text entry — and the resolver's "typing
        // always wins" rule would then swallow the next single-key binding as
        // a character typed into something nobody can see. Dropping the focus
        // first is what makes `c` after `Esc` open the composer again rather
        // than type a `c` into it.
        gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        window.shell().grab_focus();
    }

    /// Loads a draft into the fields without reporting it as an edit.
    fn fill(&self, draft: Draft) {
        let imp = self.imp();
        imp.filling.set(true);
        // Whatever this draft is replacing, filling the fields is not itself
        // an edit worth autosaving, and a timer armed for the *previous*
        // content must not fire against this one.
        self.cancel_autosave();

        imp.to.set_text(&format_list(&draft.to));
        imp.cc.set_text(&format_list(&draft.cc));
        imp.bcc.set_text(&format_list(&draft.bcc));
        imp.subject.set_text(&draft.subject);
        // The document first, then the view over it. An HTML body is parsed
        // rather than shown raw -- which is what makes a reply to an
        // HTML-only message editable at all.
        let document = document_of(&draft.body);
        // A different draft is a different history: `load` clears the
        // Editor's, so undoing across the swap cannot put one message's
        // words into another's.
        imp.body.load(document);
        imp.heading.set_text(heading(draft.kind));

        // Cc and Bcc stay out of the way until there is something in them, or
        // until the user asks for them. A composer that shows five empty
        // fields makes the common case — one recipient — look complicated.
        imp.cc_row.set_visible(!draft.cc.is_empty());
        imp.bcc_row.set_visible(!draft.bcc.is_empty());
        self.sync_more();
        // Ephemeral to this one reply action, not a property of the draft
        // itself: `open_reply` sets it right after this call returns, for a
        // fresh reply. Anything else that opens the composer — a new
        // message, resuming a saved draft — has no reply source to have
        // scanned, so there is nothing left to say about.
        self.set_tracking_domains(&[]);

        let identity = draft.identity_id;
        *imp.draft.borrow_mut() = draft;
        imp.filling.set(false);
        self.render_attachments();

        // The draft's own identity wins over the account default: an override
        // made before it was closed is still this draft's.
        if !identity.is_some_and(|id| self.select_identity(id)) {
            self.apply_identity();
        }

        // Above the quote and above the signature, which is where a reply is
        // written and where a new message starts.
        imp.body.place_caret_start();
        self.refresh();
    }

    /// Puts the keyboard where this kind of composition starts.
    ///
    /// A widget that is not mapped yet cannot take focus, and on the frame the
    /// composer opens it is not — so a failed grab is retried once the layout
    /// has run. Not a delay the user can see: it is the same frame the
    /// composer first paints in.
    ///
    /// The retry is a tick callback, not an idle: `idle_add_local_once` is not
    /// ordered against the frame clock that drives the layout pass doing the
    /// mapping, so the retry can lose that race and leave the keyboard
    /// nowhere (`postio-43`, the same shape `postio-1ff` turned out to be —
    /// see `8daa510`). A tick callback runs on the frame clock, so waiting
    /// two ticks puts the retry strictly after the layout pass; the first
    /// tick can be the one the mapping happens in.
    fn focus_first(&self) {
        let imp = self.imp();
        let field: gtk::Widget = match first_field(imp.draft.borrow().kind) {
            Field::To => imp.to.clone().upcast(),
            Field::Body => imp.body.widget().clone().upcast(),
        };
        if !field.grab_focus() {
            let ticks = Cell::new(0u8);
            field.clone().add_tick_callback(move |field, _| {
                ticks.set(ticks.get() + 1);
                if ticks.get() < 2 {
                    return glib::ControlFlow::Continue;
                }
                field.grab_focus();
                glib::ControlFlow::Break
            });
        }
    }

    /// Shows the Cc and Bcc rows. What the `+ Cc` button does.
    pub fn show_copy_fields(&self) {
        let imp = self.imp();
        imp.cc_row.set_visible(true);
        imp.bcc_row.set_visible(true);
        self.sync_more();
        imp.cc.grab_focus();
    }

    fn sync_more(&self) {
        let imp = self.imp();
        imp.more
            .set_visible(!(imp.cc_row.is_visible() && imp.bcc_row.is_visible()));
    }

    /// Redraws everything that follows from what is in the fields.
    fn refresh(&self) {
        let imp = self.imp();
        let draft = self.draft();

        match recipient_warning(&draft) {
            Some(text) => {
                imp.warning.set_text(&text);
                imp.warning.set_visible(true);
            }
            None => imp.warning.set_visible(false),
        }
        imp.send.set_sensitive(draft.is_sendable());

        if imp.filling.get() {
            return;
        }
        for handler in imp.changed.borrow().iter() {
            handler(&draft);
        }
        self.schedule_autosave();
    }

    /// Arms (or re-arms) the debounced autosave: [`Composer::save`] runs
    /// again once [`AUTOSAVE_DEBOUNCE`] passes with no further edit.
    ///
    /// A source id is not `Clone`, so re-arming always cancels whatever was
    /// pending first — there is only ever one edit's worth of waiting to do.
    fn schedule_autosave(&self) {
        // Not mounted yet: this is `build()`'s own initial layout — setting
        // the placeholder identity, for instance — not a user editing
        // anything, and there is nowhere for a save to go yet regardless.
        if self.imp().window.upgrade().is_none() {
            return;
        }
        self.cancel_autosave();
        let source = glib::timeout_add_local_once(
            AUTOSAVE_DEBOUNCE,
            glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move || {
                    composer.imp().autosave_source.set(None);
                    composer.save();
                }
            ),
        );
        self.imp().autosave_source.set(Some(source));
    }

    /// Drops any pending debounced autosave without running it — for content
    /// that is about to stop existing (a fresh draft loaded, a discard, a
    /// send), where firing it later would save the wrong thing.
    fn cancel_autosave(&self) {
        if let Some(source) = self.imp().autosave_source.take() {
            source.remove();
        }
    }

    /// Runs a pending debounced autosave immediately instead of letting it
    /// lapse, and cancels the timer that would otherwise fire it again later.
    fn flush_autosave(&self) {
        if let Some(source) = self.imp().autosave_source.take() {
            source.remove();
            self.save();
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-composer");
        self.set_accessible_role(gtk::AccessibleRole::Form);
        self.update_property(&[gtk::accessible::Property::Label("Composer")]);

        imp.heading.set_text(heading(DraftKind::New));
        imp.heading.add_css_class("postio-compose-heading");
        imp.heading.set_xalign(0.0);
        imp.heading
            .set_accessible_role(gtk::AccessibleRole::Heading);

        imp.status.add_css_class("postio-compose-status");
        imp.status.set_xalign(0.0);
        // `Status` is a live region: a screen reader announces it when it
        // changes, without moving the focus. "draft saved" is feedback, and
        // feedback only sighted users get is feedback half the users do not.
        imp.status.set_accessible_role(gtk::AccessibleRole::Status);

        imp.detach.add_css_class("flat");
        imp.detach.add_css_class("postio-ghost");
        sync_detach_button(&imp.detach, false);
        imp.detach.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.toggle_detached()
        ));

        let title = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        title.add_css_class("postio-compose-title");
        title.append(&imp.heading);
        title.append(&imp.status);
        // Pushed to the trailing edge: the heading says what this is, the
        // status says what has happened to it, and the one control up here is
        // the one that moves the whole thing somewhere else. Send and Save
        // stay at the bottom with the rest of the verbs.
        imp.status.set_hexpand(true);
        title.append(&imp.detach);

        let fields = gtk::Box::new(gtk::Orientation::Vertical, 0);
        fields.add_css_class("postio-compose-fields");
        fields.append(&self.build_to_row());
        fields.append(&self.build_row(&imp.cc_row, "Cc", &imp.cc));
        fields.append(&self.build_row(&imp.bcc_row, "Bcc", &imp.bcc));
        fields.append(&self.build_identity_row());
        fields.append(&self.build_row(
            &gtk::Box::new(gtk::Orientation::Horizontal, 14),
            "Subject",
            &imp.subject,
        ));
        imp.cc_row.set_visible(false);
        imp.bcc_row.set_visible(false);

        imp.body.widget().add_css_class("postio-compose-body");
        // The editing surface is ADR 0003's WebView, adopted for every
        // draft (ADR 0004 Q6 as amended, #347). The Editor owns the
        // document, its EditHistory and the typing-run coalescing — the
        // two-undo-stacks separation ADR 0004 Q5 draws still stands
        // (editing undo is the document's; the mail undo is `u`), it is
        // simply enforced in one place now, inside the Editor.
        imp.body.connect_changed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.body_edited()
        ));

        // Ctrl+Z / Ctrl+Shift+Z, on the body only — and at CAPTURE phase,
        // which is load-bearing: WebKit keeps an editing undo of its own on
        // a contenteditable surface, and letting the keystroke through
        // would run two histories against one document. The composer's
        // handler wins; the document's history is the only one.
        let editing = gtk::EventControllerKey::new();
        editing.set_propagation_phase(gtk::PropagationPhase::Capture);
        editing.connect_key_pressed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| {
                if !state.contains(gdk::ModifierType::CONTROL_MASK) {
                    return glib::Propagation::Proceed;
                }
                let shift = state.contains(gdk::ModifierType::SHIFT_MASK);
                match key {
                    gdk::Key::z if !shift => composer.undo_edit(),
                    gdk::Key::z | gdk::Key::Z if shift => composer.redo_edit(),
                    gdk::Key::y => composer.redo_edit(),
                    // A paste whose clipboard holds pixels is ours: WebKit
                    // would write an unresolvable fake URL into the DOM,
                    // and the whole point of #341 is that image bytes go to
                    // the blob store instead. Text pastes proceed to WebKit.
                    gdk::Key::v if !shift => {
                        if composer.paste_image() {
                            glib::Propagation::Stop
                        } else {
                            glib::Propagation::Proceed
                        }
                    }
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        imp.body.widget().add_controller(editing);

        // The body's postio-cid: requests resolve against this draft's
        // attachments, through whatever connect_attachment_bytes wired in —
        // the same path a resumed draft's images take.
        *imp.blob_lookup.borrow_mut() = Some(Box::new(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            None,
            move |content_id: &str| {
                let attachment = composer
                    .imp()
                    .draft
                    .borrow()
                    .attachments
                    .iter()
                    .find(|attachment| {
                        attachment
                            .content_id
                            .as_deref()
                            .map(|id| id.trim_start_matches('<').trim_end_matches('>'))
                            == Some(content_id)
                    })
                    .cloned()?;
                let reader = composer.imp().attachment_bytes.borrow();
                let bytes = reader.as_ref()?(&attachment)?;
                Some((bytes, attachment.mime_type.clone()))
            }
        )));
        // The WebView scrolls its own document, so there is no
        // ScrolledWindow around the body any more — one scroll surface, its
        // own tab stop, announcing the same name the TextView did.
        imp.body
            .widget()
            .update_property(&[gtk::accessible::Property::Label(BODY_NAME)]);

        // An image dropped on the body goes inline, exactly like a paste;
        // anything else falls through to the composer-wide target and
        // becomes an ordinary attachment.
        let body_drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        body_drop.connect_drop(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            false,
            move |_, value, _, _| {
                let Ok(list) = value.get::<gdk::FileList>() else {
                    return false;
                };
                let mut handled = false;
                for file in list.files() {
                    match dropped_image(&file) {
                        Some((bytes, mime_type)) => {
                            composer.add_inline_image(bytes, &mime_type);
                        }
                        None => composer.add_file(&file),
                    }
                    handled = true;
                }
                handled
            }
        ));
        imp.body.widget().add_controller(body_drop);

        imp.warning.add_css_class("postio-compose-warning");
        imp.warning.set_xalign(0.0);
        imp.warning.set_visible(false);
        imp.warning.set_accessible_role(gtk::AccessibleRole::Status);

        imp.tracking_notice
            .add_css_class("postio-compose-tracking-notice");
        imp.tracking_notice.set_xalign(0.0);
        imp.tracking_notice.set_wrap(true);
        imp.tracking_notice.set_visible(false);
        imp.tracking_notice
            .set_accessible_role(gtk::AccessibleRole::Status);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&title);
        column.append(&fields);
        column.append(&imp.tracking_notice);
        column.append(&self.build_toolbar());
        column.append(imp.body.widget());
        column.append(&imp.warning);
        column.append(&self.build_attachments());
        column.append(&self.build_actions());
        self.set_child(Some(&column));
        self.install_drop_target();

        for entry in [&imp.to, &imp.cc, &imp.bcc, &imp.subject] {
            entry.connect_changed(glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |_| composer.refresh()
            ));
        }
        // (The body's refresh rides body_edited, wired above.)

        // Recipient completion, on every field it makes sense for — not
        // `Subject`, which is not an address.
        *imp.to_completion.borrow_mut() = Some(Completion::install(self, &imp.to));
        Completion::install(self, &imp.cc);
        Completion::install(self, &imp.bcc);

        self.set_identities(Vec::new());
        self.refresh();
    }

    /// The formatting toolbar: the visible half of #338's commands.
    ///
    /// Every button dispatches the registry command the keyboard reaches —
    /// the same `dispatch` arm, so a click and `Ctrl+B` are one code path —
    /// and each toggle reflects the caret through the editor's format-state
    /// channel. Buttons keep their hands off the focus (`focus_on_click`
    /// false): the click's target is the selection in the editor, and
    /// stealing the caret would destroy the thing the command acts on.
    fn build_toolbar(&self) -> gtk::Box {
        let imp = self.imp();
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        toolbar.add_css_class("postio-compose-toolbar");
        toolbar.set_accessible_role(gtk::AccessibleRole::Toolbar);
        toolbar.update_property(&[gtk::accessible::Property::Label("Formatting")]);

        let toggles: [(CommandId, &str, &str); 5] = [
            (
                CommandId::Bold,
                "format-text-bold-symbolic",
                "postio-toolbar-bold",
            ),
            (
                CommandId::Italic,
                "format-text-italic-symbolic",
                "postio-toolbar-italic",
            ),
            (
                CommandId::BulletList,
                "view-list-bullet-symbolic",
                "postio-toolbar-bullet-list",
            ),
            (
                CommandId::NumberedList,
                "view-list-ordered-symbolic",
                "postio-toolbar-numbered-list",
            ),
            (
                CommandId::QuoteBlock,
                "format-indent-more-symbolic",
                "postio-toolbar-quote-block",
            ),
        ];
        for (button, (id, icon, class)) in imp.format_toggles.iter().zip(toggles) {
            style_toolbar_button(button.as_ref(), id, icon, class);
            button.connect_clicked(glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |_| composer.dispatch(id)
            ));
            toolbar.append(button);
        }

        style_toolbar_button(
            &imp.link_button,
            CommandId::InsertLink,
            "insert-link-symbolic",
            "postio-toolbar-link",
        );
        imp.link_button.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.dispatch(CommandId::InsertLink)
        ));
        toolbar.append(&imp.link_button);

        // Reflection: the caret's formatting drives the toggles. Programmatic
        // `set_active` emits `toggled`, not `clicked`, so nothing loops.
        let toggles = imp.format_toggles.clone();
        imp.body.connect_format_state(move |state| {
            let states = [
                state.bold,
                state.italic,
                state.bullet_list,
                state.numbered_list,
                state.quote_block,
            ];
            for (button, active) in toggles.iter().zip(states) {
                button.set_active(active);
            }
        });

        toolbar
    }

    /// The `To` row, which also carries the button that reveals Cc and Bcc.
    fn build_to_row(&self) -> gtk::Box {
        let imp = self.imp();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);

        imp.more.set_label("+ Cc");
        imp.more.add_css_class("flat");
        imp.more.add_css_class("postio-compose-more");
        imp.more
            .update_property(&[gtk::accessible::Property::Label("Show Cc and Bcc")]);
        imp.more.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.show_copy_fields()
        ));

        let row = self.build_row(&row, "To", &imp.to);
        row.append(&imp.more);
        row
    }

    /// The `From` row: the identity this draft sends as.
    fn build_identity_row(&self) -> gtk::Box {
        let imp = self.imp();
        let row = &imp.identity_row;
        row.add_css_class("postio-compose-row");
        row.append(&field_label("From"));

        imp.identity.add_css_class("postio-compose-identity");
        // Flat: the row is the control. A filled pill in the middle of four
        // hairline rows reads as the one thing on the form worth pressing,
        // which is the Send button's job.
        imp.identity.add_css_class("flat");
        imp.identity.set_hexpand(true);
        imp.identity.set_halign(gtk::Align::Start);
        imp.identity
            .update_property(&[gtk::accessible::Property::Label("Send as")]);
        imp.identity.connect_selected_notify(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.apply_identity()
        ));
        imp.identity_only.add_css_class("postio-compose-identity");
        imp.identity_only.set_xalign(0.0);
        imp.identity_only.set_hexpand(true);

        row.append(&imp.identity);
        row.append(&imp.identity_only);

        // The signature picker sits on the From row because that is where the
        // question "who is this from, and how does it sign" is answered, but
        // it is a separate control on purpose: choosing a signature must not
        // change the sending address, which is the whole point of #12.
        imp.signature.add_css_class("postio-compose-identity");
        imp.signature.add_css_class("flat");
        imp.signature.set_halign(gtk::Align::End);
        imp.signature
            .update_property(&[gtk::accessible::Property::Label("Signature")]);
        imp.signature.connect_selected_notify(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.apply_identity()
        ));
        imp.signature.set_visible(false);
        row.append(&imp.signature);
        row.clone()
    }

    /// One labelled field row, hairline-separated like the canvas draws them.
    fn build_row(&self, row: &gtk::Box, label: &str, entry: &gtk::Entry) -> gtk::Box {
        row.add_css_class("postio-compose-row");
        entry.set_hexpand(true);
        entry.add_css_class("postio-compose-field");
        entry.update_property(&[gtk::accessible::Property::Label(label)]);
        row.append(&field_label(label));
        row.append(entry);
        row.clone()
    }

    /// `Send` and `Save draft`, and the reminder of what `Esc` costs.
    ///
    /// Discard is deliberately *not* here. It is the one destructive verb in
    /// the composer, and a one-click destructive button beside Send is a
    /// misclick waiting to happen; `ctrl+d` and the palette reach it, and both
    /// ask first.
    fn build_actions(&self) -> gtk::Box {
        let imp = self.imp();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("postio-compose-actions");

        imp.send.set_child(Some(&labelled("Send", "C-Ret")));
        imp.send.add_css_class("suggested-action");
        imp.send
            .update_property(&[gtk::accessible::Property::Label("Send")]);
        imp.send.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.send()
        ));

        imp.schedule_send
            .set_child(Some(&labelled("Schedule…", "C-⇧-Ret")));
        imp.schedule_send.add_css_class("flat");
        imp.schedule_send.add_css_class("postio-ghost");
        imp.schedule_send
            .update_property(&[gtk::accessible::Property::Label("Schedule send")]);
        // Rebuilt every time the picker opens rather than once here: the
        // presets are relative to whatever moment it opens, and a composer
        // can sit unsent for a while before anyone reaches for this.
        imp.schedule_send.set_create_popup_func(|button| {
            let menu = gio::Menu::new();
            for (label, when) in schedule_presets(Local::now()) {
                let item = gio::MenuItem::new(Some(label), None);
                item.set_action_and_target_value(
                    Some("compose-schedule.choose"),
                    Some(&when.with_timezone(&Utc).timestamp_millis().to_variant()),
                );
                menu.append_item(&item);
            }
            button.set_popover(Some(&gtk::PopoverMenu::from_model(Some(&menu))));
        });

        let schedule_actions = gio::SimpleActionGroup::new();
        let choose = gio::SimpleAction::new("choose", Some(glib::VariantTy::INT64));
        choose.connect_activate(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_, parameter| {
                let Some(millis) = parameter.and_then(|value| value.get::<i64>()) else {
                    return;
                };
                if let Some(when) = DateTime::<Utc>::from_timestamp_millis(millis) {
                    composer.send_later(when);
                }
            }
        ));
        schedule_actions.add_action(&choose);
        imp.schedule_send
            .insert_action_group("compose-schedule", Some(&schedule_actions));

        imp.save.set_child(Some(&labelled("Save draft", "C-s")));
        imp.save.add_css_class("flat");
        imp.save.add_css_class("postio-ghost");
        imp.save
            .update_property(&[gtk::accessible::Property::Label("Save draft")]);
        imp.save.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.save()
        ));

        imp.escape.set_label("Esc keeps the draft");
        imp.escape.add_css_class("postio-compose-escape");
        imp.escape.set_hexpand(true);
        imp.escape.set_xalign(1.0);
        // #692: the least essential of the four, so it is the one that
        // gives way under a narrow allocation. Without this, nothing here
        // can shrink below its natural width, and the whole row overflows
        // the window instead -- clipping this label against the window's
        // own edge rather than showing anything legible.
        imp.escape.set_ellipsize(gtk::pango::EllipsizeMode::End);

        row.append(&imp.send);
        row.append(&imp.schedule_send);
        row.append(&imp.save);
        row.append(&imp.escape);
        row
    }

    // -- Test support -----------------------------------------------------

    /// Sets the subject field as if the user had typed it, firing the
    /// entry's own `changed` signal.
    ///
    /// This crate's GTK integration tests are a separate compilation unit
    /// from `src/composer.rs`'s own `#[cfg(test)]` module (GTK needs one
    /// process per display, so each lives in its own file — see
    /// `tests/gtk_composer.rs`) and so cannot reach a private field to
    /// exercise what a keystroke triggers. Not meant for anything but tests.
    #[doc(hidden)]
    pub fn test_set_subject(&self, text: &str) {
        self.imp().subject.set_text(text);
    }

    /// What the subject field is showing. Not meant for anything but tests.
    #[doc(hidden)]
    pub fn test_subject(&self) -> String {
        self.imp().subject.text().to_string()
    }

    /// Where the cursor is in the body, as a character offset.
    ///
    /// The acceptance criterion for detaching is that it keeps the cursor,
    /// and the only way to state that as an assertion is to be able to read
    /// it. Not meant for anything but tests.
    #[doc(hidden)]
    pub fn test_cursor_offset(&self) -> i32 {
        self.imp().body.caret_offset()
    }

    /// Puts the keyboard in `field`, as clicking into it would.
    ///
    /// Detaching claims to keep the focus where it was, and a test cannot
    /// state that without first putting it somewhere other than where a fresh
    /// composition would have left it. Not meant for anything but tests.
    #[doc(hidden)]
    pub fn test_focus_field(&self, field: Field) -> bool {
        match field {
            Field::To => self.imp().to.grab_focus(),
            Field::Body => self.imp().body.widget().grab_focus(),
        }
    }

    /// The pop-out button, so a test can assert the pointer has a way in too.
    #[doc(hidden)]
    pub fn test_detach_button(&self) -> gtk::Button {
        self.imp().detach.clone()
    }

    /// The "Schedule send…" button, so a test can assert the keyboard's way
    /// into the picker (`CommandId::ScheduleSend`) is the same one the
    /// pointer has.
    #[doc(hidden)]
    pub fn test_schedule_send_button(&self) -> gtk::MenuButton {
        self.imp().schedule_send.clone()
    }

    /// Whether the action row's trailing hint is allowed to give way under a
    /// narrow allocation (#692) -- the least essential of the row's four
    /// elements, so it is the one that should, rather than a button's own
    /// label losing a word or the row overflowing the window outright.
    #[doc(hidden)]
    pub fn test_escape_hint_ellipsizes(&self) -> bool {
        self.imp().escape.ellipsize() == gtk::pango::EllipsizeMode::End
    }

    /// Types `text` into the body, the way a keystroke reaches the buffer.
    #[doc(hidden)]
    pub fn test_set_body(&self, text: &str) {
        self.imp().body.test_type(text);
    }

    /// Feeds image bytes into the paste path without a clipboard, which a
    /// headless test cannot reliably own.
    #[doc(hidden)]
    pub fn test_paste_image_bytes(&self, bytes: Vec<u8>) {
        self.add_inline_image(bytes, "image/png");
    }

    /// Script against the body's document, for assertions about what the
    /// surface actually rendered.
    #[doc(hidden)]
    pub fn test_body_eval(&self, script: &str) -> String {
        self.imp().body.test_eval(script)
    }

    /// Choose the signature at `index` in the picker, as a click would.
    #[doc(hidden)]
    pub fn test_choose_signature(&self, index: u32) {
        self.imp().signature.set_selected(index);
    }

    /// Select a range of the body's `nth` text node, as a hand would before
    /// reaching for a formatting button.
    #[doc(hidden)]
    pub fn test_select_body(&self, nth: u32, from: u32, to: u32) {
        self.imp().body.test_select(nth, from, to);
    }

    /// The draft `Reply` would open for `source`, without a window to press
    /// a key in. Public so the quoting path can be driven directly.
    #[doc(hidden)]
    pub fn test_reply_draft(&self, source: &Message) -> Option<Draft> {
        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        reply_draft(CommandId::Reply, source, &account)
    }

    /// Types `text` into `To`, firing recipient completion the same way the
    /// entry's own `changed` signal does for a real keystroke.
    #[doc(hidden)]
    pub fn test_set_to(&self, text: &str) {
        self.imp().to.set_text(text);
    }

    /// Whether `To`'s completion popover is currently showing suggestions.
    #[doc(hidden)]
    pub fn test_recipient_popover_visible(&self) -> bool {
        self.imp()
            .to_completion
            .borrow()
            .as_ref()
            .is_some_and(|completion| completion.popover.is_visible())
    }

    /// Accepts `To`'s currently-selected suggestion, exactly as `Enter` would
    /// — without synthesizing a real key event, which a test cannot easily do
    /// for a specific widget's own key controller.
    #[doc(hidden)]
    pub fn test_accept_recipient_suggestion(&self) -> bool {
        let imp = self.imp();
        let Some(completion) = imp.to_completion.borrow().clone() else {
            return false;
        };
        completion.accept(&imp.to)
    }

    /// Activates the `index`th suggestion the way a mouse click does, by
    /// emitting the `row-activated` that `GtkListBox` emits for a click on a
    /// row. Nothing about the button press is simulated — this is the signal
    /// the click produces, so anything listening for the click hears this.
    #[doc(hidden)]
    pub fn test_click_recipient_suggestion(&self, index: usize) -> bool {
        let Some(completion) = self.imp().to_completion.borrow().clone() else {
            return false;
        };
        let Some(row) = completion.list.row_at_index(index as i32) else {
            return false;
        };
        completion.list.select_row(Some(&row));
        completion.list.emit_by_name::<()>("row-activated", &[&row]);
        true
    }

    /// Delivers `key` to the completion's key handling, and reports whether it
    /// was consumed.
    ///
    /// This calls the handler the entry's key controller calls; it does not
    /// synthesize a GDK event, which GTK4 gives a test no way to do for a
    /// particular widget. So it covers what the handler does with a key and
    /// *not* whether a real keystroke reaches the handler at all — the
    /// propagation phase decides that, and only running the app shows it.
    #[doc(hidden)]
    pub fn test_press_recipient_key(&self, key: gdk::Key) -> bool {
        let imp = self.imp();
        let Some(completion) = imp.to_completion.borrow().clone() else {
            return false;
        };
        completion.handle_key(&imp.to, key) == glib::Propagation::Stop
    }

    /// How many suggestions `To` is currently offering.
    #[doc(hidden)]
    pub fn test_recipient_suggestion_count(&self) -> usize {
        self.imp()
            .to_completion
            .borrow()
            .as_ref()
            .map_or(0, |completion| completion.candidates.borrow().len())
    }

    /// Attaches `path` exactly as [`CommandId::AttachFile`] or a drop would,
    /// without going through `GtkFileDialog` or GTK's drag machinery — a
    /// headless test can drive neither. Both real paths converge on the same
    /// `add_file`, so this exercises what they exercise.
    #[doc(hidden)]
    pub fn test_attach_path(&self, path: &std::path::Path) {
        self.add_file(&gio::File::for_path(path));
    }

    /// How many attachments the draft currently carries.
    #[doc(hidden)]
    pub fn test_attachment_count(&self) -> usize {
        self.imp().draft.borrow().attachments.len()
    }

    /// Whether the attachment list is showing at all.
    #[doc(hidden)]
    pub fn test_attachments_visible(&self) -> bool {
        self.imp().attachments_box.is_visible()
    }

    /// Removes the attachment at `index`, as its row's own button would.
    #[doc(hidden)]
    pub fn test_remove_attachment(&self, index: usize) {
        self.remove_attachment(index);
    }
}

/// Installs a composer in `window` and returns it.
///
/// One call, because there is nothing to choose: the composer belongs in the
/// reading pane of the window that owns the keyboard.
pub fn install(window: &Window) -> Composer {
    let composer = Composer::new();
    composer.mount(window);
    composer
}

/// The other children of the reading pane — what the composer takes over from.
/// Whether `focus` is `widget` or something inside it.
/// Dress one toolbar button: icon, identity, and a tooltip that teaches the
/// key — title and binding both read from the registry, so the toolbar can
/// never drift from what the palette and the cheat sheet say.
fn style_toolbar_button(button: &gtk::Button, id: CommandId, icon: &str, class: &str) {
    let spec = postio_core::registry::get(id);
    button.set_icon_name(icon);
    button.add_css_class("flat");
    button.add_css_class(class);
    button.set_tooltip_text(Some(&format!(
        "{} ({})",
        spec.title,
        pretty_binding(spec.default_binding)
    )));
    button.update_property(&[gtk::accessible::Property::Label(spec.title)]);
    // The click acts on the editor's selection; taking the focus would
    // collapse the very thing the command is for.
    button.set_focus_on_click(false);
}

/// `ctrl+shift+8` the way a tooltip says it: `Ctrl+Shift+8`.
fn pretty_binding(binding: &str) -> String {
    binding
        .split('+')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// `file`'s bytes and MIME type when it is an image small enough to inline,
/// `None` to send it down the ordinary attachment path instead.
///
/// The type comes from the shared-mime-info sniff, not the extension; the
/// size cap keeps a stray drop of a camera RAW from ballooning the message —
/// past it, the file is still attached, just not inlined.
fn dropped_image(file: &gio::File) -> Option<(Vec<u8>, String)> {
    const INLINE_CAP: u64 = 10 * 1024 * 1024;
    let info = file
        .query_info(
            "standard::content-type,standard::size",
            gio::FileQueryInfoFlags::NONE,
            gio::Cancellable::NONE,
        )
        .ok()?;
    let mime_type = info.content_type()?.to_string();
    if !mime_type.starts_with("image/") || info.size() as u64 > INLINE_CAP {
        return None;
    }
    let path = file.path()?;
    let bytes = std::fs::read(path).ok()?;
    Some((bytes, mime_type))
}

fn holds(focus: &gtk::Widget, widget: &gtk::Widget) -> bool {
    focus == widget || focus.is_ancestor(widget)
}

/// How an identity reads in the `From` row: the name, then the address.
fn identity_label(identity: &Identity) -> String {
    if identity.display_name.trim().is_empty() || identity.display_name == identity.address.address
    {
        identity.address.address.clone()
    } else {
        format!("{} <{}>", identity.display_name, identity.address.address)
    }
}

/// A field's name, in the mono face the canvas sets metadata in.
fn field_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-compose-label");
    label.set_xalign(0.0);
    label.set_width_chars(8);
    // The entry it labels already carries the same name.
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

/// A human size for an attachment row: `812 B`, `48 KB`, `3.2 MB`.
fn format_size(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    let bytes = bytes as f64;
    if bytes >= MIB {
        format!("{:.1} MB", bytes / MIB)
    } else if bytes >= KIB {
        format!("{:.0} KB", bytes / KIB)
    } else {
        format!("{bytes:.0} B")
    }
}

/// A preset must land at least this far ahead of `now` to be offered as
/// "today" rather than rolling to tomorrow — a picker opened one minute
/// before 6pm must not offer "this evening" for an instant already gone.
const MIN_SCHEDULE_LEAD: Duration = Duration::minutes(5);

/// `day` at the given wall-clock hour and minute, in `day`'s own local zone.
///
/// A DST transition can make a wall-clock time ambiguous or nonexistent;
/// falling back to `day` itself rather than panicking keeps a schedule-send
/// picker from crashing the composer on the two days a year this can happen,
/// at the cost of an odd-looking preset on exactly those days.
fn at_local_time(day: DateTime<Local>, hour: u32, minute: u32) -> DateTime<Local> {
    day.date_naive()
        .and_hms_opt(hour, minute, 0)
        .and_then(|naive| naive.and_local_timezone(Local).single())
        .unwrap_or(day)
}

/// The fixed times [`CommandId::ScheduleSend`]'s picker offers, computed
/// against `now` — recomputed every time the picker opens rather than once,
/// since "in 1 hour" a picker opened yesterday is not "in 1 hour" today.
///
/// "This evening" rolls to tomorrow once 6pm today is behind `now`.
/// "Monday morning" always means a Monday strictly after today: opening the
/// picker on a Monday offers next week's, not the one already underway.
fn schedule_presets(now: DateTime<Local>) -> [(&'static str, DateTime<Local>); 4] {
    let in_one_hour = now + Duration::hours(1);

    let mut evening = at_local_time(now, 18, 0);
    if evening < now + MIN_SCHEDULE_LEAD {
        evening = at_local_time(now + Duration::days(1), 18, 0);
    }

    let tomorrow_morning = at_local_time(now + Duration::days(1), 8, 0);

    let days_from_monday = now.weekday().num_days_from_monday() as i64;
    let days_until_monday = if days_from_monday == 0 {
        7
    } else {
        7 - days_from_monday
    };
    let monday_morning = at_local_time(now + Duration::days(days_until_monday), 8, 0);

    [
        ("In 1 hour", in_one_hour),
        ("This evening", evening),
        ("Tomorrow morning", tomorrow_morning),
        ("Monday morning", monday_morning),
    ]
}

/// A button label with the key that reaches it, as the header bar does it.
fn labelled(text: &str, key: &str) -> gtk::Widget {
    let label = gtk::Label::new(Some(text));
    let hint = gtk::Label::new(Some(key));
    hint.add_css_class("postio-keyhint");
    hint.set_accessible_role(gtk::AccessibleRole::Presentation);

    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.append(&label);
    row.append(&hint);
    row.upcast()
}

/// Redraws the header's `Compose` button for whether the composer has the
/// reading pane. The button never stops naming `win.compose` — see
/// `mount`'s action handler for what that does in each state — this only
/// changes what it says while it does it.
fn sync_compose_button(button: &gtk::Button, composing: bool) {
    let (icon, text, key, tooltip) = if composing {
        (
            "window-close-symbolic",
            "Composing",
            "Esc",
            "Close the composer",
        )
    } else {
        (
            "document-edit-symbolic",
            "Compose",
            "c",
            "Compose a message",
        )
    };

    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    content.append(&gtk::Image::from_icon_name(icon));
    content.append(&labelled(text, key));
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
}

/// Keeps the pop-out button saying which way it goes.
///
/// One control rather than two, for the same reason the header's Compose
/// button doubles as Close: a button that is only correct while the composer
/// is in one of its two places is a button that lies half the time.
fn sync_detach_button(button: &gtk::Button, detached: bool) {
    let (icon, tooltip) = if detached {
        (
            "view-restore-symbolic",
            "Put the composer back in the reading pane",
        )
    } else {
        (
            "window-new-symbolic",
            "Write in a window of its own, so you can read something else",
        )
    };
    button.set_child(Some(&gtk::Image::from_icon_name(icon)));
    button.set_tooltip_text(Some(tooltip));
    button.update_property(&[gtk::accessible::Property::Label(tooltip)]);
}

/// How much of the recipient being typed must exist before completion offers
/// anything.
///
/// Four, from #424. One character matches most of an address book, so the
/// popover opened over the field with a list nobody could choose from yet —
/// and it did it while a query ran on every keystroke. Four is where a prefix
/// starts to identify somebody.
const MIN_COMPLETION_PREFIX: usize = 4;

/// Recipient completion attached to one entry: a popover of suggestions from
/// [`Composer::connect_recipient_suggestions`], keyboard-navigable and
/// accepted without ever reaching for the mouse.
///
/// One per entry (`To`, `Cc`, `Bcc`) rather than one shared instance, because
/// a `Popover` is parented to the one widget it points at.
pub(crate) struct Completion {
    popover: gtk::Popover,
    list: gtk::ListBox,
    /// What `list`'s rows currently show, in the same order, so accepting a
    /// selected row can look up what it stands for.
    candidates: RefCell<Vec<RecipientCandidate>>,
}

impl Completion {
    /// Wires completion onto `entry`. The returned value is not meant to be
    /// kept: `entry`'s own signal connections hold it alive for as long as
    /// the entry exists, which for a composer field is the app's lifetime.
    fn install(composer: &Composer, entry: &gtk::Entry) -> Rc<Self> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Browse);
        list.add_css_class("postio-recipient-suggestions");
        list.set_accessible_role(gtk::AccessibleRole::ListBox);

        let popover = gtk::Popover::new();
        // Autohide would take the grab away from the entry the moment the
        // popover opens, and a completion list that steals the keyboard from
        // what is still being typed into is worse than no completion at all.
        popover.set_autohide(false);
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.add_css_class("postio-recipient-completion");
        popover.set_child(Some(&list));
        popover.set_parent(entry);

        let this = Rc::new(Self {
            popover,
            list,
            candidates: RefCell::new(Vec::new()),
        });

        entry.connect_changed(glib::clone!(
            #[weak]
            composer,
            #[strong]
            this,
            move |entry| this.update(&composer, entry)
        ));

        // A click commits the row it landed on. Without this the list looked
        // interactive and was not: clicking moved GTK's own selection and
        // nothing ever acted on it, so the only way to take a suggestion was
        // the keyboard (#424).
        this.list.connect_row_activated(glib::clone!(
            #[strong]
            this,
            #[weak]
            entry,
            move |_, row| {
                this.accept_row(&entry, row);
            }
        ));

        let keys = gtk::EventControllerKey::new();
        // Capture, not the default bubble.
        //
        // The widget that actually holds the focus is the `GtkText` inside
        // the entry, and `GtkText` binds Return to its `activate` keybinding,
        // which consumes the event. A bubble-phase controller on the entry
        // runs *after* the focus widget has had it, so Return never arrived
        // here: the popover could be walked with the arrow keys and then not
        // accepted with the keyboard, which is what #424 reported. Capturing
        // takes the key on the way down, before the text widget's own
        // bindings run, and `handle_key` still declines everything it does
        // not want and every key at all while the popover is closed.
        //
        // The same reasoning, and the same setting, as every other key
        // controller in this app: `window.rs`, `finder.rs`, `cheatsheet.rs`,
        // `settings.rs`.
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[strong]
            this,
            #[weak]
            entry,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| this.handle_key(&entry, key)
        ));
        entry.add_controller(keys);

        this
    }

    /// Re-searches for whatever is now being typed, and shows or hides the
    /// popover to match.
    fn update(&self, composer: &Composer, entry: &gtk::Entry) {
        let text = entry.text();
        let (_, token) = current_entry(&text);
        // Below the threshold nothing is offered *and* nothing is looked up:
        // the provider is a database query, and running one per keystroke to
        // rank half the address book on one letter costs something and tells
        // the user nothing. Counted in `chars`, because a threshold measured
        // in bytes would ask more of a name written in one script than
        // another.
        if token.chars().count() < MIN_COMPLETION_PREFIX {
            self.popover.popdown();
            return;
        }
        let candidates = {
            let provider = composer.imp().recipient_suggestions.borrow();
            match provider.as_ref() {
                Some(provider) => provider(token),
                None => return,
            }
        };
        self.populate(candidates);
    }

    fn populate(&self, candidates: Vec<RecipientCandidate>) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        for candidate in &candidates {
            let label = gtk::Label::new(Some(&candidate_label(candidate)));
            label.set_xalign(0.0);
            self.list.append(&label);
        }
        let empty = candidates.is_empty();
        *self.candidates.borrow_mut() = candidates;

        if empty {
            self.popover.popdown();
        } else {
            self.list.select_row(self.list.row_at_index(0).as_ref());
            if !self.popover.is_visible() {
                self.popover.popup();
            }
        }
    }

    /// Handles the keys that only mean something while the popover is open;
    /// everything else falls through to the entry as normal.
    fn handle_key(&self, entry: &gtk::Entry, key: gdk::Key) -> glib::Propagation {
        if !self.popover.is_visible() {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Down => {
                self.move_selection(1);
                glib::Propagation::Stop
            }
            gdk::Key::Up => {
                self.move_selection(-1);
                glib::Propagation::Stop
            }
            gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::Tab => {
                if self.accept(entry) {
                    glib::Propagation::Stop
                } else {
                    glib::Propagation::Proceed
                }
            }
            gdk::Key::Escape => {
                self.popover.popdown();
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    }

    fn move_selection(&self, delta: i32) {
        let count = self.candidates.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = self.list.selected_row().map(|row| row.index()).unwrap_or(0);
        let next = (current + delta).rem_euclid(count);
        if let Some(row) = self.list.row_at_index(next) {
            self.list.select_row(Some(&row));
        }
    }

    /// Replaces the token being typed with the selected suggestion, leaving
    /// every address typed before it untouched, and leaves a `, ` in place
    /// for whatever the user types next.
    fn accept(&self, entry: &gtk::Entry) -> bool {
        let Some(row) = self.list.selected_row() else {
            return false;
        };
        self.accept_row(entry, &row)
    }

    /// Commits one specific row, whether the keyboard selected it or a click
    /// landed on it. A click carries the row it hit, so this takes the row
    /// rather than re-reading the selection — the two agree today, and a
    /// click that committed something other than what was under the pointer
    /// would be the worst possible way for them ever to disagree.
    fn accept_row(&self, entry: &gtk::Entry, row: &gtk::ListBoxRow) -> bool {
        let Some(candidate) = self.candidates.borrow().get(row.index() as usize).cloned() else {
            return false;
        };

        // A contact inserts one address; a group inserts every member as its
        // own address, comma by comma, exactly as if they had been typed
        // individually -- there is no group reference to insert instead
        // (ADR 0007 Q3).
        let inserted: String = match &candidate {
            RecipientCandidate::Contact(address) => format!("{address}, "),
            RecipientCandidate::Group { members, .. } => members
                .iter()
                .map(|address| format!("{address}, "))
                .collect(),
        };

        let text = entry.text();
        let (start, _) = current_entry(&text);
        let mut replaced = text.to_string();
        replaced.replace_range(start.., &inserted);
        entry.set_text(&replaced);
        entry.set_position(-1);
        self.popover.popdown();
        true
    }
}

/// The completion row's label: an address for a contact, or the name and
/// size for a group -- distinguishable from a contact at a glance, since
/// accepting one inserts several addresses rather than one.
fn candidate_label(candidate: &RecipientCandidate) -> String {
    match candidate {
        RecipientCandidate::Contact(address) => address.to_string(),
        RecipientCandidate::Group { name, members } => {
            format!("{name} ({} people)", members.len())
        }
    }
}

/// The document a draft's stored body means.
///
/// HTML is the record (ADR 0004 Q3), so a draft that has one is parsed from
/// it: the typed form is derived, never stored. Falling back to the text half
/// matters for every draft written before this existed, and for the plain
/// messages that never grow an HTML half at all.
fn document_of(body: &MessageBody) -> postio_body::Document {
    match (&body.html, &body.text) {
        (Some(html), _) => postio_body::parse(html),
        (None, Some(text)) => postio_body::Document::from_text(text),
        (None, None) => postio_body::Document::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use postio_model::EmailAddress;

    fn draft() -> Draft {
        Draft::new(AccountId::UNASSIGNED)
    }

    #[test]
    fn a_reply_starts_in_the_body_and_new_mail_starts_in_to() {
        for kind in [DraftKind::New, DraftKind::Forward] {
            assert_eq!(first_field(kind), Field::To, "{kind:?}");
        }
        for kind in [DraftKind::Reply, DraftKind::ReplyAll] {
            assert_eq!(first_field(kind), Field::Body, "{kind:?}");
        }
    }

    #[test]
    fn the_heading_names_the_composition_in_the_app_s_own_words() {
        assert_eq!(heading(DraftKind::New), "Compose");
        assert_eq!(heading(DraftKind::Reply), "Reply");
        assert_eq!(heading(DraftKind::ReplyAll), "Reply all");
        assert_eq!(heading(DraftKind::Forward), "Forward");
    }

    #[test]
    fn closing_keeps_anything_the_user_typed() {
        assert_eq!(closing(&draft()), Closing::Drop);

        let mut with_recipient = draft();
        with_recipient.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        assert_eq!(closing(&with_recipient), Closing::Keep);

        let mut with_subject = draft();
        with_subject.subject = "the mbox importer".to_owned();
        assert_eq!(closing(&with_subject), Closing::Keep);

        let mut with_body = draft();
        with_body.body = MessageBody {
            text: Some("looking now".to_owned()),
            html: None,
        };
        assert_eq!(closing(&with_body), Closing::Keep);
    }

    #[test]
    fn closing_drops_a_draft_holding_only_the_signature_the_composer_added() {
        let mut signed = draft();
        signed.body = MessageBody {
            text: Some("\n\n-- \nAda\n".to_owned()),
            html: None,
        };
        assert_eq!(closing(&signed), Closing::Drop);

        signed.body = MessageBody {
            text: Some("Looking now.\n\n-- \nAda\n".to_owned()),
            html: None,
        };
        assert_eq!(
            closing(&signed),
            Closing::Keep,
            "a word above it is content"
        );
    }

    #[test]
    fn closing_drops_a_draft_that_only_ever_held_whitespace() {
        let mut blank = draft();
        blank.subject = "   ".to_owned();
        blank.body = MessageBody {
            text: Some("\n\n".to_owned()),
            html: None,
        };
        assert_eq!(closing(&blank), Closing::Drop);
    }

    #[test]
    fn implausible_recipients_are_counted_not_refused() {
        let mut one = draft();
        one.to = vec![
            EmailAddress::new(None::<String>, "ada@example.com"),
            EmailAddress::new(None::<String>, "grace"),
        ];
        assert_eq!(
            recipient_warning(&one).as_deref(),
            Some("1 address does not look like an address")
        );

        one.cc = vec![EmailAddress::new(None::<String>, "@example.com")];
        assert_eq!(
            recipient_warning(&one).as_deref(),
            Some("2 addresses do not look like addresses")
        );

        let mut fine = draft();
        fine.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        assert_eq!(recipient_warning(&fine), None);
    }

    fn source_message() -> Message {
        let mut message = Message::new(
            AccountId::new(1),
            postio_model::ids::MailboxId::new(1),
            chrono::Utc::now(),
        );
        message.id = postio_model::ids::MessageId::new(42);
        message.from = vec![EmailAddress::new(Some("Ada Lovelace"), "ada@example.com")];
        message.to = vec![EmailAddress::new(None::<String>, "grace@example.com")];
        message.subject = Some("Quarterly numbers".to_owned());
        message
    }

    fn account_reading_it() -> Account {
        let mut account = Account::new(
            "Test",
            EmailAddress::new(None::<String>, "grace@example.com"),
        );
        account.id = AccountId::new(1);
        account
    }

    #[test]
    fn each_reply_command_maps_to_its_own_draft_kind() {
        let source = source_message();
        let account = account_reading_it();

        let reply = reply_draft(CommandId::Reply, &source, &account).expect("a draft");
        assert_eq!(reply.kind, DraftKind::Reply);
        assert_eq!(reply.subject, "Re: Quarterly numbers");

        let reply_all = reply_draft(CommandId::ReplyAll, &source, &account).expect("a draft");
        assert_eq!(reply_all.kind, DraftKind::ReplyAll);

        let forward = reply_draft(CommandId::Forward, &source, &account).expect("a draft");
        assert_eq!(forward.kind, DraftKind::Forward);
        assert_eq!(forward.subject, "Fwd: Quarterly numbers");
    }

    #[test]
    fn a_command_that_is_not_a_reply_maps_to_nothing() {
        let source = source_message();
        let account = account_reading_it();
        assert!(reply_draft(CommandId::Archive, &source, &account).is_none());
    }

    // ── issue #116: the quoted-tracking-link banner ────────────────────────

    #[test]
    fn a_subdomain_of_the_sender_is_not_flagged_but_an_unrelated_host_is() {
        assert!(!differs_from_sender("shop.example.org", "shop.example.org"));
        assert!(!differs_from_sender(
            "click.shop.example.org",
            "shop.example.org"
        ));
        assert!(!differs_from_sender("SHOP.EXAMPLE.ORG", "shop.example.org"));
        assert!(differs_from_sender(
            "click.tracker.example.org",
            "shop.example.org"
        ));
        // Not a suffix match on the bare string: "notshop.example.org" is a
        // different domain than "shop.example.org", not a subdomain of it.
        assert!(differs_from_sender(
            "notshop.example.org",
            "shop.example.org"
        ));
    }

    #[test]
    fn the_notice_names_one_domain_or_counts_several() {
        assert_eq!(tracking_link_notice(&[]), None);
        assert_eq!(
            tracking_link_notice(&["click.tracker.example.org".to_owned()]).as_deref(),
            Some(
                "The quoted text links to click.tracker.example.org, which differs \
                 from the sender's own domain."
            )
        );
        assert_eq!(
            tracking_link_notice(&["a.example.org".to_owned(), "b.example.org".to_owned()])
                .as_deref(),
            Some(
                "The quoted text links to 2 domains that differ from the sender's \
                 own: a.example.org, b.example.org."
            )
        );
    }

    /// The `.eml` corpus fixture #116 itself points at: an HTML-only message
    /// from `orders@shop.example.org` linking out to
    /// `click.tracker.example.org`.
    fn html_only_source_with_a_tracking_link() -> Message {
        let mut message = Message::new(
            AccountId::new(1),
            postio_model::ids::MailboxId::new(1),
            chrono::Utc::now(),
        );
        message.from = vec![EmailAddress::new(
            Some("Cooperage Supply"),
            "orders@shop.example.org",
        )];
        message.body = MessageBody {
            text: None,
            html: Some(
                "<p><a href=\"https://click.tracker.example.org/r?u=abc&c=aa71\">\
                 Shop now</a></p>"
                    .to_owned(),
            ),
        };
        message
    }

    #[test]
    fn an_html_only_reply_source_with_a_foreign_link_is_flagged() {
        let source = html_only_source_with_a_tracking_link();
        assert_eq!(
            quoted_tracking_domains(&source),
            ["click.tracker.example.org"]
        );
    }

    #[test]
    fn a_link_to_the_senders_own_domain_is_not_flagged() {
        let mut source = html_only_source_with_a_tracking_link();
        source.body.html =
            Some("<p><a href=\"https://shop.example.org/catalog\">Shop now</a></p>".to_owned());
        assert!(quoted_tracking_domains(&source).is_empty());
    }

    #[test]
    fn a_reply_to_a_rich_message_quotes_its_structure_not_flattened_text() {
        // #340: the quote is the parsed document, so the editor opens on a
        // real Quote block with the source's structure inside it — and the
        // text half still reads as the `> ` convention.
        let mut source = html_only_source_with_a_tracking_link();
        source.body = MessageBody {
            text: Some("The lamp has shipped".to_owned()),
            html: Some("<p>The lamp <strong>has shipped</strong></p>".to_owned()),
        };
        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        let draft = reply_draft(CommandId::Reply, &source, &account).expect("a reply");

        let document = document_of(&draft.body);
        assert!(
            document.blocks.iter().any(|block| matches!(
                block,
                postio_body::Block::Quote(blocks) if blocks.iter().any(|inner| matches!(
                    inner,
                    postio_body::Block::Paragraph(inlines) if inlines
                        .iter()
                        .any(|inline| matches!(inline, postio_body::Inline::Strong(_)))
                ))
            )),
            "{document:?}"
        );
        let text = draft.body.text.expect("a text half");
        assert!(text.contains("> The lamp"), "{text}");
        assert!(text.contains("wrote:"), "{text}");
    }

    #[test]
    fn replying_to_your_own_flowed_plain_text_message_unwraps_the_soft_breaks() {
        // #456: a message this app sent as plain text only, format=flowed,
        // whose sentence happened to wrap across three physical lines --
        // exactly `plain-text-flowed-reply.eml`'s shape. Quoting it must
        // read as the one sentence the sender wrote, not three lines they
        // never typed.
        let mut source = Message::new(
            AccountId::new(1),
            postio_model::ids::MailboxId::new(1),
            chrono::Utc::now(),
        );
        source.from = vec![EmailAddress::new(
            Some("Quinn Abara"),
            "quinn.abara@example.net",
        )];
        let sentence = "That order works for me, though I would rather take the \
                         sign-off question first while everyone is still in the room \
                         and awake, because the layout argument tends to eat the \
                         whole hour once it starts.";
        source.body = MessageBody {
            text: Some(
                "That order works for me, though I would rather take the sign-off \n\
                 question first while everyone is still in the room and awake, because \n\
                 the layout argument tends to eat the whole hour once it starts."
                    .to_owned(),
            ),
            html: None,
        };
        source.text_is_flowed = true;

        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        let draft = reply_draft(CommandId::Reply, &source, &account).expect("a reply");

        let document = document_of(&draft.body);
        let quoted_paragraph_is_one_unbroken_sentence = document.blocks.iter().any(|block| {
            matches!(
                block,
                postio_body::Block::Quote(blocks) if blocks.iter().any(|inner| matches!(
                    inner,
                    postio_body::Block::Paragraph(inlines)
                        if inlines.as_slice() == [postio_body::Inline::Text(sentence.to_owned())]
                ))
            )
        });
        assert!(
            quoted_paragraph_is_one_unbroken_sentence,
            "the quote must be the sender's one sentence, not their soft \
             wrap read back as typed line breaks: {document:?}"
        );
    }

    #[test]
    fn replying_to_an_ordinary_senders_short_lines_leaves_them_as_typed() {
        // The other half of #456's acceptance: a message from someone else
        // that merely has short lines, and no format=flowed parameter at
        // all, must not be reflowed -- real intentional line breaks would
        // get eaten.
        let mut source = Message::new(
            AccountId::new(1),
            postio_model::ids::MailboxId::new(1),
            chrono::Utc::now(),
        );
        source.from = vec![EmailAddress::new(
            Some("Ada Norwood"),
            "ada.norwood@example.com",
        )];
        source.body = MessageBody {
            text: Some("Short note before the walkthrough.\nSee you then.".to_owned()),
            html: None,
        };
        source.text_is_flowed = false;

        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        let draft = reply_draft(CommandId::Reply, &source, &account).expect("a reply");

        let document = document_of(&draft.body);
        let quote_keeps_both_typed_lines = document.blocks.iter().any(|block| {
            matches!(
                block,
                postio_body::Block::Quote(blocks) if blocks.iter().any(|inner| matches!(
                    inner,
                    postio_body::Block::Paragraph(inlines) if inlines.as_slice() == [
                        postio_body::Inline::Text("Short note before the walkthrough.".to_owned()),
                        postio_body::Inline::Break,
                        postio_body::Inline::Text("See you then.".to_owned()),
                    ]
                ))
            )
        });
        assert!(
            quote_keeps_both_typed_lines,
            "an ordinary sender's own line break must survive as a break, \
             not be joined onto its neighbour: {document:?}"
        );
    }

    #[test]
    fn a_forward_of_a_hostile_message_carries_no_load_and_no_script() {
        // ADR 0003 hardening requirement 6, at the wiring level: the draft a
        // forward opens with is built from the closed document type, so the
        // pixel and the script have no representation in either half.
        let mut source = html_only_source_with_a_tracking_link();
        source.subject = Some("Your order".to_owned());
        source.body.html = Some(
            r#"<p>Your order <b>has shipped</b>. <img src="https://pixel.tracker.example.org/o.gif" width="1" height="1"> <script>document.location='https://evil.example.org'</script></p>"#
                .to_owned(),
        );
        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        let draft = reply_draft(CommandId::Forward, &source, &account).expect("a forward");

        let html = draft.body.html.expect("a forward is rich");
        let text = draft.body.text.expect("and has a text half");
        for leak in [
            "<img",
            "<script",
            "pixel.tracker.example.org",
            "evil.example.org",
        ] {
            assert!(
                !html.contains(leak),
                "{leak} leaked:
{html}"
            );
            assert!(
                !text.contains(leak),
                "{leak} leaked:
{text}"
            );
        }
        assert!(html.contains("has shipped"), "{html}");
        assert!(text.contains("Forwarded message"), "{text}");
        assert!(text.contains("Subject: Your order"), "{text}");
    }

    #[test]
    fn a_message_with_both_parts_is_scanned_because_the_html_is_what_gets_quoted() {
        // Before #340, a text part meant the reply quoted that text and the
        // HTML's links were never in it. The rich quote is built from the
        // markup the reader showed, so its links are exactly what the reply
        // re-sends — and what the notice must name.
        let mut source = html_only_source_with_a_tracking_link();
        source.body.text = Some("Shop now: see the HTML version".to_owned());
        assert_eq!(
            quoted_tracking_domains(&source),
            ["click.tracker.example.org"]
        );
    }

    #[test]
    fn a_message_with_no_html_at_all_is_never_scanned() {
        let mut source = html_only_source_with_a_tracking_link();
        source.body.html = None;
        assert!(quoted_tracking_domains(&source).is_empty());
    }

    // -- Schedule send presets ---------------------------------------------

    fn local_at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Local> {
        Local
            .with_ymd_and_hms(year, month, day, hour, minute, 0)
            .single()
            .expect("an unambiguous local time")
    }

    fn preset(presets: &[(&'static str, DateTime<Local>); 4], label: &str) -> DateTime<Local> {
        presets
            .iter()
            .find(|(name, _)| *name == label)
            .unwrap_or_else(|| panic!("no `{label}` preset"))
            .1
    }

    #[test]
    fn an_ordinary_morning_offers_the_same_evening_and_tomorrow() {
        // 2024-01-01 is a Monday.
        let now = local_at(2024, 1, 1, 9, 0);
        let presets = schedule_presets(now);

        assert_eq!(preset(&presets, "In 1 hour"), local_at(2024, 1, 1, 10, 0));
        assert_eq!(
            preset(&presets, "This evening"),
            local_at(2024, 1, 1, 18, 0),
            "6pm is still ahead of a 9am picker"
        );
        assert_eq!(
            preset(&presets, "Tomorrow morning"),
            local_at(2024, 1, 2, 8, 0)
        );
        assert_eq!(
            preset(&presets, "Monday morning"),
            local_at(2024, 1, 8, 8, 0),
            "today is already Monday, so the preset means next week's"
        );
    }

    #[test]
    fn this_evening_rolls_to_tomorrow_once_this_evening_has_passed() {
        let now = local_at(2024, 1, 1, 19, 0);
        let presets = schedule_presets(now);

        assert_eq!(
            preset(&presets, "This evening"),
            local_at(2024, 1, 2, 18, 0),
            "6pm today is behind a 7pm picker"
        );
    }

    #[test]
    fn monday_morning_is_the_nearest_monday_still_ahead() {
        // 2024-01-03 is a Wednesday.
        let now = local_at(2024, 1, 3, 8, 0);
        let presets = schedule_presets(now);

        assert_eq!(
            preset(&presets, "Monday morning"),
            local_at(2024, 1, 8, 8, 0)
        );
    }

    #[test]
    fn every_preset_is_ahead_of_now() {
        for now in [
            local_at(2024, 1, 1, 0, 5),
            local_at(2024, 1, 1, 17, 59),
            local_at(2024, 1, 1, 23, 55),
        ] {
            for (label, when) in schedule_presets(now) {
                assert!(when > now, "{label} is not ahead of {now}: {when}");
            }
        }
    }
}
