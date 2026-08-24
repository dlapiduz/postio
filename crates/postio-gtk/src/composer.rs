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
use gtk::{gdk, glib};
use postio_core::{CommandId, Context};
use postio_model::address::{current_entry, format_list, parse_list};
use postio_model::{
    Account, AccountId, Attachment, Draft, DraftKind, EmailAddress, Identity, IdentityId, Message,
    MessageBody,
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
pub fn first_field(kind: DraftKind) -> Field {
    match kind {
        DraftKind::New => Field::To,
        DraftKind::Reply | DraftKind::ReplyAll | DraftKind::Forward => Field::Body,
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

/// What the status line says before anything has happened to the draft.
/// What the body field announces itself as. One constant because the scroll
/// region around it is a separate tab stop and must say the same thing.
const BODY_NAME: &str = "Message body";

const UNSAVED: &str = "draft is in the composer only";

/// What the status line says when opening compose came back to a kept draft.
const RETAINED: &str = "still holding the draft Esc kept";

/// What the status line says when [`CommandId::Send`] has nowhere to go.
const NO_SEND_PATH: &str = "not sent — no outgoing account is connected yet";

/// What the status line says when a file was chosen or dropped but nothing
/// is listening on [`Composer::connect_attach`] to turn it into an attachment.
const NO_ATTACH_PATH: &str = "not attached — no attachment handler is connected yet";

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

/// Answers "what does `prefix` complete to" for recipient completion —
/// contacts and previous correspondents, ranked by frequency and recency, are
/// [`Composer::connect_recipient_suggestions`]'s job to supply; the composer
/// only shows what it is given, in the order it is given.
type RecipientSuggestions = Box<dyn Fn(&str) -> Vec<EmailAddress>>;

/// Answers "what message is `e`/`E`/`f` about" — the composer has no notion
/// of a reading pane or a selection of its own. `None` means there is nothing
/// to reply to right now (nothing open, or nothing to send as), in which case
/// the keystroke does nothing rather than opening a broken composer.
type ReplySourceProvider = Box<dyn Fn() -> Option<(Message, Account)>>;

/// What [`Composer::connect_attach`] hands its result to, exactly once:
/// `Some` with the finished attachment, `None` to reject the file (unreadable,
/// say). Calling this is what actually adds the row — synchronously, for a
/// handler that already has the answer, or from a spawned task's own
/// callback, for one that had to go read the file first. `pub` because it
/// appears in `connect_attach`'s public signature, not because anything
/// outside this module constructs one.
pub type AttachReady = Box<dyn FnOnce(Option<Attachment>)>;

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
    let source = quotable(source);
    let source = source.as_ref();
    match id {
        CommandId::Reply => Some(reply::reply(source, account)),
        CommandId::ReplyAll => Some(reply::reply_all(source, account)),
        CommandId::Forward => Some(reply::forward(source, account)),
        _ => None,
    }
}

/// Give `source` a text body when all it has is markup.
///
/// `postio_model::reply` quotes plain text, and it cannot do otherwise:
/// `postio-model` sits *below* `postio-body` in the layering — the body crate
/// depends on the model, so the model cannot reach the parser without a
/// cycle. Turning markup into text is therefore done here, in the crate that
/// has both, and handed down as an ordinary text body.
///
/// Before this, replying to an HTML-only message produced an attribution line
/// with nothing under it. Marketing mail, calendar invitations and anything
/// composed in a webmail client are HTML-only, so "nothing to quote" was the
/// common case rather than an edge one.
///
/// Borrowed unless there is something to add, so the ordinary path does not
/// clone a message to change nothing about it.
fn quotable(source: &Message) -> std::borrow::Cow<'_, Message> {
    use std::borrow::Cow;

    let has_text = source
        .body
        .text
        .as_deref()
        .is_some_and(|text| !text.trim().is_empty());
    if has_text {
        return Cow::Borrowed(source);
    }
    let Some(html) = source.body.html.as_deref() else {
        return Cow::Borrowed(source);
    };
    // `to_text` over the closed subset, never a general HTML-to-text pass:
    // that is the function that makes most mail's plain-text part
    // unreadable, and the reason `postio_body::Document` is a small closed
    // set in the first place.
    let text = postio_body::parse(html).to_text();
    if text.trim().is_empty() {
        return Cow::Borrowed(source);
    }
    let mut owned = source.clone();
    owned.body.text = Some(text);
    Cow::Owned(owned)
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
        pub identity: gtk::DropDown,
        pub identity_only: gtk::Label,
        /// The editor surface. A **view** over [`Composer::document`], never
        /// the body's state — see ADR 0004. Its own undo is turned off in
        /// `build`, deliberately.
        pub body: gtk::TextView,
        /// The body, as the model every frontend shares.
        ///
        /// `GtkTextBuffer`, `NSTextStorage` and a `contenteditable` DOM
        /// disagree about what a rich text document is, so a composer whose
        /// body state is "whatever is in the buffer" makes a second
        /// frontend's composer a rewrite rather than a port. This is the
        /// document; the widget draws it.
        pub document: RefCell<postio_body::Document>,
        /// Editing undo, over the document. Not `postio_core::undo`, which is
        /// the *mail* undo bound to `u`; see ADR 0004 Q5.
        pub history: RefCell<postio_body::EditHistory>,
        /// The document the history believes is on screen, so a change knows
        /// what it changed *from*.
        pub baseline: RefCell<postio_body::Document>,
        /// When the last edit landed, for coalescing a typing run into one
        /// undo step rather than one per keystroke.
        pub last_edit: Cell<Option<std::time::Instant>>,
        /// Set while undo or redo is writing the buffer, so the change it
        /// causes is not recorded as a fresh edit on top of itself.
        pub restoring: Cell<bool>,
        pub send: gtk::Button,
        pub save: gtk::Button,
        pub warning: gtk::Label,
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
        /// Where recipient completion gets its candidates. One slot: the
        /// same search serves `To`, `Cc` and `Bcc` alike.
        pub recipient_suggestions: RefCell<Option<RecipientSuggestions>>,
        /// Where a chosen or dropped file becomes attachment metadata. One
        /// slot: the same handler serves the file chooser and drag-and-drop.
        pub attach: RefCell<Option<AttachHandler>>,
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
                identity: gtk::DropDown::from_strings(&[]),
                identity_only: gtk::Label::new(None),
                body: gtk::TextView::new(),
                document: RefCell::new(postio_body::Document::new()),
                history: RefCell::new(postio_body::EditHistory::new()),
                baseline: RefCell::new(postio_body::Document::new()),
                last_edit: Cell::new(None),
                restoring: Cell::new(false),
                send: gtk::Button::new(),
                save: gtk::Button::new(),
                warning: gtk::Label::new(None),
                attachments_box: gtk::Box::new(gtk::Orientation::Vertical, 6),
                attachments_list: gtk::ListBox::new(),
                draft: RefCell::new(Draft::new(AccountId::UNASSIGNED)),
                identities: RefCell::new(Vec::new()),
                sent: RefCell::new(Vec::new()),
                saved: RefCell::new(Vec::new()),
                changed: RefCell::new(Vec::new()),
                closed: RefCell::new(Vec::new()),
                opened: RefCell::new(Vec::new()),
                reply_source: RefCell::new(None),
                recipient_suggestions: RefCell::new(None),
                attach: RefCell::new(None),
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
        } else if holds(&focus, imp.body.upcast_ref()) {
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
    fn body(&self) -> MessageBody {
        let document = self.document();
        let text = document.to_text();
        MessageBody {
            text: (!text.is_empty()).then_some(text),
            html: (!document.is_plain_text()).then(|| postio_body::render(&document).1),
        }
    }

    /// Fold the surface's current state into the editing history.
    ///
    /// Coalescing by time rather than by keystroke count: what a person means
    /// by one undo is "the thing I was just typing", and that is a pause in
    /// the typing, not a number of characters.
    fn record_edit(&self) {
        let imp = self.imp();
        // Filling the fields is not an edit, and neither is undo writing the
        // buffer -- recording that would push the state it just restored back
        // on top of the stack and make Ctrl+Z a no-op.
        if imp.filling.get() || imp.restoring.get() {
            return;
        }
        let current = self.document();
        let now = std::time::Instant::now();
        let continuing = imp
            .last_edit
            .get()
            .is_some_and(|last| now.duration_since(last) < EDIT_COALESCE);

        let mut history = imp.history.borrow_mut();
        if !(continuing && history.amend(current.clone())) {
            let before = imp.baseline.borrow().clone();
            history.record(before, current.clone());
        }
        drop(history);
        *imp.baseline.borrow_mut() = current.clone();
        *imp.document.borrow_mut() = current;
        imp.last_edit.set(Some(now));
    }

    /// Step the body back one edit.
    fn undo_edit(&self) -> glib::Propagation {
        let restored = self.imp().history.borrow_mut().undo();
        self.restore(restored)
    }

    /// Step it forward again.
    fn redo_edit(&self) -> glib::Propagation {
        let restored = self.imp().history.borrow_mut().redo();
        self.restore(restored)
    }

    /// Put `document` on screen without it counting as a new edit.
    fn restore(&self, document: Option<postio_body::Document>) -> glib::Propagation {
        let Some(document) = document else {
            // Nothing to step to. Swallowed rather than propagated: Ctrl+Z at
            // the start of the history must not fall through to whatever else
            // in the window would answer it.
            return glib::Propagation::Stop;
        };
        let imp = self.imp();
        imp.restoring.set(true);
        imp.body.buffer().set_text(&document.to_text());
        *imp.baseline.borrow_mut() = document.clone();
        *imp.document.borrow_mut() = document;
        imp.restoring.set(false);
        // A restored state is a settled one: the next keystroke starts a new
        // step rather than amending the one just undone.
        imp.last_edit.set(None);
        glib::Propagation::Stop
    }

    /// The body as it stands, with the editor's edits folded in.
    ///
    /// v1 edits plain text, so reading the surface back means rebuilding a
    /// plain-text document -- which is exactly what a plain-text composer
    /// does, and why ADR 0004 says the model needs no restricting for v1,
    /// only the editor. A rich surface replaces this method and nothing else.
    pub fn document(&self) -> postio_body::Document {
        let buffer = self.imp().body.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        postio_body::Document::from_text(&text)
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

        let buffer = imp.body.buffer();
        let current = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        let wanted = draft.body.text.unwrap_or_default();
        if current == wanted {
            return;
        }

        // The written part above the signature is untouched, so the caret
        // belongs exactly where it was — switching identity mid-sentence must
        // not send the cursor to the end of the message.
        let caret = buffer.cursor_position();
        let was_filling = imp.filling.replace(true);
        buffer.set_text(&wanted);
        let caret = caret.min(buffer.char_count());
        buffer.place_cursor(&buffer.iter_at_offset(caret));
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
    pub fn send(&self) {
        if self.imp().sent.borrow().is_empty() {
            self.set_status(NO_SEND_PATH);
            return;
        }
        let draft = self.draft();
        for handler in self.imp().sent.borrow().iter() {
            handler(&draft);
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

    /// Registers what completes a recipient prefix — contacts and previous
    /// correspondents, ranked and searched however the caller sees fit. The
    /// composer only shows what comes back, in that order, and never touches
    /// the network itself: purely local completion is the whole point.
    pub fn connect_recipient_suggestions(
        &self,
        provider: impl Fn(&str) -> Vec<EmailAddress> + 'static,
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
                imp.body.grab_focus();
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
        self.set_visible(false);
        reader.append(self);

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
                    composer.open(Draft::new(composer.account()));
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
            CommandId::Compose => self.open(Draft::new(self.account())),
            CommandId::Send if self.is_open() => self.send(),
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
            _ => {}
        }
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
        if let Some(draft) = reply_draft(id, &source, &account) {
            self.open(draft);
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

        for sibling in pane_siblings(self) {
            sibling.set_visible(false);
        }
        // In the one-pane mode the reader is not necessarily on screen, and a
        // composer the user cannot see is the worst possible mode.
        shell.set_focused_pane(Pane::Reader);
        shell.add_css_class(COMPOSING_CLASS);
        window.set_context(Context::Composer);
    }

    /// Puts the reading pane back the way it was.
    fn release_pane(&self) {
        for sibling in pane_siblings(self) {
            sibling.set_visible(true);
        }
        let Some(window) = self.imp().window.upgrade() else {
            return;
        };
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
        imp.body.buffer().set_text(&document.to_text());
        *imp.baseline.borrow_mut() = document.clone();
        *imp.document.borrow_mut() = document;
        // A different draft is a different history. Undoing across the swap
        // would put one message's words into another's.
        imp.history.borrow_mut().clear();
        imp.last_edit.set(None);
        imp.heading.set_text(heading(draft.kind));

        // Cc and Bcc stay out of the way until there is something in them, or
        // until the user asks for them. A composer that shows five empty
        // fields makes the common case — one recipient — look complicated.
        imp.cc_row.set_visible(!draft.cc.is_empty());
        imp.bcc_row.set_visible(!draft.bcc.is_empty());
        self.sync_more();

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
        let buffer = imp.body.buffer();
        buffer.place_cursor(&buffer.start_iter());
        self.refresh();
    }

    /// Puts the keyboard where this kind of composition starts.
    ///
    /// A widget that is not mapped yet cannot take focus, and on the frame the
    /// composer opens it is not — so a failed grab is retried once the layout
    /// has run. Not a delay the user can see: it is the same frame the
    /// composer first paints in.
    fn focus_first(&self) {
        let imp = self.imp();
        let field: gtk::Widget = match first_field(imp.draft.borrow().kind) {
            Field::To => imp.to.clone().upcast(),
            Field::Body => imp.body.clone().upcast(),
        };
        if !field.grab_focus() {
            glib::idle_add_local_once(move || {
                field.grab_focus();
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

        imp.body.add_css_class("postio-compose-body");
        imp.body.set_wrap_mode(gtk::WrapMode::WordChar);
        imp.body.set_vexpand(true);
        // Off, explicitly, and this comment is the point of the line.
        //
        // `GtkTextBuffer`'s own undo is free and it is the wrong undo. A
        // `GtkTextBuffer` step is a typing run in a flat buffer; a
        // `contenteditable` step is whatever the browser's editing command
        // coalesced. Take the free one and the GTK composer and a future
        // macOS or web composer disagree about what one Ctrl+Z does — which
        // is precisely the class of divergence ADR 0004 exists to prevent,
        // and leaving it on is the *silent* way to get it, because it works
        // until there is a second frontend to disagree with.
        //
        // Editing undo is a `postio_body::EditHistory` over `Document`, and
        // it is a different stack from `postio_core::undo`, which is the
        // *mail* undo bound to `u` and carries its inverse as commands.
        imp.body.buffer().set_enable_undo(false);

        // Every change to the surface becomes a step on the document's
        // history. Recorded here rather than from a key handler because an
        // edit is an edit however it arrived -- typed, pasted, or dropped in.
        imp.body.buffer().connect_changed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.record_edit()
        ));

        // Ctrl+Z / Ctrl+Shift+Z, on the body only. The mail undo is `u` and
        // lives in `Context::Normal`; the registry already refuses to bind
        // the mail verbs in `Context::Composer`, and this is the other half
        // of keeping the two stacks apart.
        let editing = gtk::EventControllerKey::new();
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
                    _ => glib::Propagation::Proceed,
                }
            }
        ));
        imp.body.add_controller(editing);
        // Tab moves to the next control rather than typing a tab: the body is
        // in the middle of the focus chain, and a keyboard user has to be able
        // to get out of it.
        imp.body.set_accepts_tab(false);
        imp.body
            .update_property(&[gtk::accessible::Property::Label(BODY_NAME)]);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&imp.body)
            .build();
        // The scroll area is a tab stop in its own right — a keyboard has to
        // be able to scroll it — so it is reached before the body itself and
        // has to announce the same thing rather than nothing.
        scroller.update_property(&[gtk::accessible::Property::Label(BODY_NAME)]);

        imp.warning.add_css_class("postio-compose-warning");
        imp.warning.set_xalign(0.0);
        imp.warning.set_visible(false);
        imp.warning.set_accessible_role(gtk::AccessibleRole::Status);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&title);
        column.append(&fields);
        column.append(&scroller);
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
        imp.body.buffer().connect_changed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.refresh()
        ));

        // Recipient completion, on every field it makes sense for — not
        // `Subject`, which is not an address.
        *imp.to_completion.borrow_mut() = Some(Completion::install(self, &imp.to));
        Completion::install(self, &imp.cc);
        Completion::install(self, &imp.bcc);

        self.set_identities(Vec::new());
        self.refresh();
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

        let escape = gtk::Label::new(Some("Esc keeps the draft"));
        escape.add_css_class("postio-compose-escape");
        escape.set_hexpand(true);
        escape.set_xalign(1.0);

        row.append(&imp.send);
        row.append(&imp.save);
        row.append(&escape);
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
        let buffer = self.imp().body.buffer();
        buffer.iter_at_mark(&buffer.get_insert()).offset()
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
            Field::Body => self.imp().body.grab_focus(),
        }
    }

    /// The pop-out button, so a test can assert the pointer has a way in too.
    #[doc(hidden)]
    pub fn test_detach_button(&self) -> gtk::Button {
        self.imp().detach.clone()
    }

    /// Types `text` into the body, the way a keystroke reaches the buffer.
    #[doc(hidden)]
    pub fn test_set_body(&self, text: &str) {
        self.imp().body.buffer().set_text(text);
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
fn pane_siblings(composer: &Composer) -> Vec<gtk::Widget> {
    let Some(pane) = composer.parent() else {
        return Vec::new();
    };
    let this: gtk::Widget = composer.clone().upcast();
    let mut siblings = Vec::new();
    let mut child = pane.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget != this {
            siblings.push(widget);
        }
    }
    siblings
}

/// Whether `focus` is `widget` or something inside it.
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
    /// selected row can look up the address it stands for.
    candidates: RefCell<Vec<EmailAddress>>,
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

        let keys = gtk::EventControllerKey::new();
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
        if token.is_empty() {
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

    fn populate(&self, candidates: Vec<EmailAddress>) {
        while let Some(row) = self.list.row_at_index(0) {
            self.list.remove(&row);
        }
        for address in &candidates {
            let label = gtk::Label::new(Some(&address.to_string()));
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
        let Some(address) = self.candidates.borrow().get(row.index() as usize).cloned() else {
            return false;
        };

        let text = entry.text();
        let (start, _) = current_entry(&text);
        let mut replaced = text.to_string();
        replaced.replace_range(start.., &format!("{address}, "));
        entry.set_text(&replaced);
        entry.set_position(-1);
        self.popover.popdown();
        true
    }
}

/// How long a pause ends a typing run, for editing undo.
///
/// Long enough that ordinary typing is one step, short enough that going
/// away and coming back is not. The same order as every editor's.
const EDIT_COALESCE: std::time::Duration = std::time::Duration::from_millis(700);

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
    use postio_model::EmailAddress;

    fn draft() -> Draft {
        Draft::new(AccountId::UNASSIGNED)
    }

    #[test]
    fn a_reply_starts_in_the_body_and_new_mail_starts_in_to() {
        assert_eq!(first_field(DraftKind::New), Field::To);
        for kind in [DraftKind::Reply, DraftKind::ReplyAll, DraftKind::Forward] {
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
}
