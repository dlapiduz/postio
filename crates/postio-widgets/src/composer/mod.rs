//! The composer: canvas 2a, which replaces the reading pane (not a window) so the message list
//! keeps its scroll and selection.
//! `Esc` closes it and keeps the draft; discarding is a separate destructive verb. It edits a
//! `Draft` and hands it back through
//! `connect_*` seams; with no handler connected it says so on the status line rather than emptying
//! the draft.

pub mod editor;
pub mod scheme;
pub mod web_process;

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use adw::prelude::*;
use adw::subclass::prelude::*;
use chrono::{DateTime, Local, Utc};
use gtk::{gdk, gio, glib};
use postio_body::Placement;
use postio_core::{CommandId, Keymap};
use postio_model::address::{current_entry, format_list, parse_list};
use postio_model::{
    Account, AccountId, Attachment, Draft, DraftKind, EmailAddress, Identity, IdentityId, Message,
    MessageBody, Signature, SignatureId,
};

use crate::widgets::keyhint;
use postio_ui::hints;

/// A field the keyboard can be in, named so the focus order can be asserted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    To,
    Cc,
    Bcc,
    Subject,
    Body,
}

/// Forward has no recipient until the user picks one, so it belongs with `New`, not the replies
/// (#690).
pub fn first_field(kind: DraftKind) -> Field {
    match kind {
        DraftKind::New | DraftKind::Forward => Field::To,
        DraftKind::Reply | DraftKind::ReplyAll => Field::Body,
    }
}

pub fn starting_field(draft: &Draft) -> Field {
    let addressed = !draft.to.is_empty() && !draft.subject.trim().is_empty();
    if addressed {
        Field::Body
    } else {
        first_field(draft.kind)
    }
}

/// The vocabulary is the app's: **Compose**, never "New" or "Write".
pub fn heading(kind: DraftKind) -> &'static str {
    match kind {
        DraftKind::New => "Compose",
        DraftKind::Reply => "Reply",
        DraftKind::ReplyAll => "Reply all",
        DraftKind::Forward => "Forward",
    }
}

pub use postio_model::draft::{Closing, closing, closing_since};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opening {
    Fill,
    Restore,
}

/// The draft asked for always wins: a reply opens the reply, `c` a blank message (#1196). A kept
/// draft is autosaved into
/// Drafts, so starting fresh loses nothing and [`Composer::resume`] reopens it. `c` while the
/// composer is already open
/// still returns focus, in [`Composer::open`]'s first branch.
pub fn opening(_kept: &Draft, _asked: &Draft) -> Opening {
    Opening::Fill
}

/// A warning, never a refusal: refusing the keystroke would lose what was typed.
pub fn recipient_warning(draft: &Draft) -> Option<String> {
    let wrong: Vec<String> = fields(draft)
        .into_iter()
        .flat_map(|(name, addresses)| {
            addresses
                .iter()
                .filter(|address| !address.is_plausible())
                .map(move |address| format!("{} in {name}", address.address))
                .collect::<Vec<_>>()
        })
        .collect();

    match wrong.len() {
        0 => None,
        // Named, not counted: a count alone leaves the user to find the wrong one among nine.
        _ => {
            let shown: Vec<&str> = wrong
                .iter()
                .take(NAMED_ADDRESSES)
                .map(String::as_str)
                .collect();
            let listed = shown.join(", ");
            if wrong.len() <= NAMED_ADDRESSES {
                Some(format!("{listed} does not look like an address"))
            } else {
                Some(format!(
                    "{listed} and {} more do not look like addresses ({} in all)",
                    wrong.len() - shown.len(),
                    wrong.len()
                ))
            }
        }
    }
}

const NAMED_ADDRESSES: usize = 3;

/// The count says which field ("1 To, 41 Cc"): a reply-all to a large list looks like a reply until
/// sent. `None` for the ordinary case.
pub fn recipient_summary(draft: &Draft) -> Option<String> {
    // The wording and threshold are `postio_ui::compose`'s, shared with the macOS composer.
    postio_ui::compose::recipient_summary(draft.to.len(), draft.cc.len(), draft.bcc.len())
}

fn fields(draft: &Draft) -> [(&'static str, &[EmailAddress]); 3] {
    [
        ("To", draft.to.as_slice()),
        ("Cc", draft.cc.as_slice()),
        ("Bcc", draft.bcc.as_slice()),
    ]
}

/// Exact match or a subdomain counts as the same sender domain.
fn differs_from_sender(host: &str, sender_domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let sender_domain = sender_domain.to_ascii_lowercase();
    host != sender_domain && !host.ends_with(&format!(".{sender_domain}"))
}

/// `None` for nothing to say. No link is ever stripped or rewritten (a link is not a load); this
/// only decides whether to say something (#116).
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

/// A tracking redirect's query string usually encodes the first recipient's id, so a reply-all's
/// other readers would click
/// through under it. Informational only (#116). Scans the same [`source_document`] the reply is
/// built from, so only quoted links count.
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

/// One constant: the scroll region around the body is a separate tab stop and must say the same
/// thing.
const BODY_NAME: &str = "Message body";

const UNSAVED: &str = postio_ui::compose::NOT_SAVED;

const RETAINED: &str = "still holding the draft Esc kept";

/// Reachable only with no account configured; the composition root connects the send handler
/// otherwise.
const NO_SEND_PATH: &str = "not sent — no outgoing account is connected yet";

/// `open` refuses to replace a retained or in-progress draft and must say so: a refusal that looked
/// like the key doing nothing was the bug (#426).
const REPLY_BLOCKED: &str = "not opened — finish or close the current draft first";

const NO_ATTACH_PATH: &str = "not attached — no attachment handler is connected yet";

const SIGNATURE_FROM_IDENTITY: &str = "Identity signature";

const NO_INLINE_PATH: &str = "not inserted — no inline-image handler is connected yet";

/// A fixed default until the configurable threshold is wired from `postio-config`, which this crate
/// does not touch.
const LARGE_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// Long enough to coalesce a burst of keystrokes into one save, short enough that a crash loses
/// little.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Everything the composer asks its app for goes through here, so the composer names no app.
pub trait ComposerHost {
    fn parent(&self) -> Option<gtk::Window>;
    fn install(&self, composer: &Composer);
    fn restore(&self, composer: &Composer);
    fn remove(&self, composer: &Composer);
    /// The detached window shows this instead of the composer alone when the host draws the
    /// composer inside a frame. `None` for a host with no frame.
    fn frame(&self) -> Option<gtk::Widget> {
        None
    }
    fn take_pane(&self);
    fn release_pane(&self);
    fn command_for_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        window: &gtk::Window,
    ) -> Option<CommandId>;
    fn handle_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        window: &gtk::Window,
    ) -> glib::Propagation;
    fn add_action(&self, action: &gio::SimpleAction);
    fn connect_command(&self, handler: Box<dyn Fn(CommandId)>);
    fn keymap(&self) -> Keymap;
}

/// Logical pixels. Narrower than the main window: a pop-out the size of its parent reads as a
/// second application.
const DETACHED_SIZE: (i32, i32) = (620, 560);

type DraftHandler = Box<dyn Fn(&Draft)>;

/// Carries the chosen time: nothing downstream can ask the picker again once the fields are
/// cleared.
type SendLaterHandler = Box<dyn Fn(&Draft, DateTime<Utc>)>;

/// `&mut`, unlike [`DraftHandler`]: the handler assigns a [`DraftId`](postio_model::DraftId) on
/// first persist and
/// [`Composer::save`] writes it back, so later saves update the same row.
type SaveHandler = Box<dyn Fn(&mut Draft)>;

type ClosedHandler = Box<dyn Fn(Closing)>;

type OpenedHandler = Box<dyn Fn()>;

pub use postio_model::contact_group::RecipientCandidate;

/// The composer only shows what it is given, in the order given.
type RecipientSuggestions = Box<dyn Fn(&str) -> Vec<RecipientCandidate>>;

/// `None` means there is nothing to reply to. The answer comes through a callback because finding
/// the message is a store
/// read the drawing thread must not wait for (#1608).
type ReplySourceProvider = Box<dyn Fn(ReplyAnswer)>;

pub type ReplyAnswer = Box<dyn FnOnce(Option<(Message, Account)>)>;

/// `None` means neither the mailbox nor the account has an opinion. Answered through a callback for
/// the reason given on
/// [`ReplySourceProvider`] (#1608).
type SignatureDefaultProvider = Box<dyn Fn(SignatureAnswer)>;

pub type SignatureAnswer = Box<dyn FnOnce(Option<SignatureId>)>;

/// `Some` with the finished attachment, `None` to reject the file. Calling it adds the row,
/// synchronously or from a spawned
/// task. `pub` because it appears in `connect_attach`'s signature.
pub type AttachReady = Box<dyn FnOnce(Option<Attachment>)>;

/// As [`AttachReady`], carrying a `Content-ID` and `Disposition::Inline`. `pub` because it appears
/// in a public signature.
pub type InlineImageReady = Box<dyn FnOnce(Option<Attachment>)>;

type InlineImageHandler = Box<dyn Fn(Vec<u8>, String, InlineImageReady)>;

type BlobLookup = Rc<RefCell<Option<Box<dyn Fn(&str) -> Option<(Vec<u8>, String)>>>>>;

/// Synchronous and local, like [`crate::reader::BlobSource`].
type AttachmentBytes = Box<dyn Fn(&Attachment) -> Option<Vec<u8>>>;

/// A handler that blocks reading a large file blocks the composer; slow work belongs on a thread or
/// task the handler spawns.
type AttachHandler = Box<dyn Fn(std::path::PathBuf, AttachReady)>;

fn reply_draft(id: CommandId, source: &Message, account: &Account) -> Option<Draft> {
    let kind = match id {
        CommandId::Reply => ReplyKind::Reply,
        CommandId::ReplyAll => ReplyKind::ReplyAll,
        CommandId::Forward => ReplyKind::Forward,
        _ => return None,
    };
    Some(postio_body::replying::reply_draft(kind, source, account))
}

use postio_body::replying::{ReplyKind, source_document};

use postio_ui::recipients::{MIN_COMPLETION_PREFIX, candidate_label};
use postio_ui::schedule::schedule_presets;
use postio_ui::sending::{ALREADY_QUEUED, NO_RECIPIENTS, join_with_and, send_concerns};

mod imp {
    use super::*;

    pub struct Composer {
        pub heading: gtk::Label,
        pub status: gtk::Label,
        pub to: gtk::Entry,
        /// `None` means no limit is configured, not unknown.
        pub size_limit: Cell<Option<u64>>,
        pub cc: gtk::Entry,
        pub bcc: gtk::Entry,
        pub subject: gtk::Entry,
        pub cc_row: gtk::Box,
        pub bcc_row: gtk::Box,
        pub more: gtk::Button,
        pub detach: gtk::Button,
        /// `Some` is the whole of "detached": a separate flag could drift from the window.
        pub detached: RefCell<Option<adw::Window>>,
        pub identity_row: gtk::Box,
        /// Entry 0 is the identity's own; the rest are the account's set.
        pub signatures: RefCell<Vec<Signature>>,
        pub signature: gtk::DropDown,
        pub identity: gtk::DropDown,
        pub identity_only: gtk::Label,
        /// The `Editor` owns the document, edit history and typing-run coalescing; the composer
        /// holds a view over its record (ADR 0004).
        pub body: editor::Editor,
        pub format_toggles: [gtk::ToggleButton; 6],
        pub link_button: gtk::Button,
        pub image_button: gtk::Button,
        pub attach_button: gtk::Button,
        pub send: gtk::Button,
        /// Its popover is rebuilt on every open (`set_create_popup_func` in `build_actions`):
        /// presets are relative to when it opens.
        pub schedule_send: gtk::MenuButton,
        pub save: gtk::Button,
        /// The least essential of the four (#692): it ellipsizes under a narrow allocation rather
        /// than overflowing the row.
        pub escape: gtk::Label,
        pub attach_hint: gtk::Box,
        pub warning: gtk::Label,
        /// Separate from `warning`: the two can be true at once.
        pub tracking_notice: gtk::Label,
        pub attachments_box: gtk::Box,
        pub below_body: gtk::Box,
        pub attachments_list: gtk::ListBox,
        /// Not part of the draft: what the fetch is doing; a new composition starts with none.
        pub downloading: RefCell<std::collections::BTreeSet<String>>,
        pub draft: RefCell<Draft>,
        /// The account a fresh composition starts from, kept apart from the draft cell: a reply
        /// fills the cell with the
        /// answered message's account, which must not become where the next new message comes from
        /// (#1161).
        pub composing_from: Cell<AccountId>,
        pub identities: RefCell<Vec<Identity>>,
        pub sent: RefCell<Vec<DraftHandler>>,
        pub sent_later: RefCell<Vec<SendLaterHandler>>,
        pub saved: RefCell<Vec<SaveHandler>>,
        pub changed: RefCell<Vec<DraftHandler>>,
        pub closed: RefCell<Vec<ClosedHandler>>,
        /// Called when a fresh `open()` takes the pane, not on the no-op path where the composer
        /// was already open.
        pub opened: RefCell<Vec<OpenedHandler>>,
        /// One slot, last registration wins: there is exactly one reading pane.
        pub reply_source: RefCell<Option<ReplySourceProvider>>,
        /// Read when `c` starts a draft, since the selected mailbox changes far more often than the
        /// account.
        pub signature_default: RefCell<Option<SignatureDefaultProvider>>,
        /// One slot: the same search serves `To`, `Cc` and `Bcc`.
        pub recipient_suggestions: RefCell<Option<RecipientSuggestions>>,
        /// New mail has no quote, so placement means nothing there.
        pub signature_placement: Cell<(Placement, Placement)>,
        /// One slot: the same handler serves the file chooser and drag-and-drop.
        pub attach: RefCell<Option<AttachHandler>>,
        pub inline_image: RefCell<Option<InlineImageHandler>>,
        pub attachment_bytes: RefCell<Option<AttachmentBytes>>,
        /// Filled by `build()` with a draft-aware lookup; held here because the editor is
        /// constructed before the composer exists.
        pub blob_lookup: BlobLookup,
        pub host: RefCell<Option<Rc<dyn super::ComposerHost>>>,
        /// Which composition is in the fields: bumped on every fill, so an answer that arrives
        /// after the composer moved on is ignored (#1608).
        pub generation: Cell<u64>,
        /// The composition before the last fill: `close` and `send` refill the composer before the
        /// closed handlers run.
        pub previous_generation: Cell<u64>,
        pub filling: Cell<bool>,
        /// Forwarded messages folded out of the editor; the body still holds them below what is
        /// typed.
        pub held: RefCell<Vec<postio_body::Block>>,
        /// The composition as the last fill left it, once signed and quoted: what "untouched" is
        /// measured against on closing.
        pub began_as: RefCell<Option<Draft>>,
        pub autosave_source: Cell<Option<glib::SourceId>>,
        /// Held here so tests can reach it.
        pub(crate) to_completion: RefCell<Option<Rc<Completion>>>,
        pub fields: gtk::Box,
        pub chrome: RefCell<Vec<gtk::Widget>>,
        pub chips: RefCell<Option<[Rc<crate::widgets::recipients::RecipientChips>; 3]>>,
    }

    impl Default for Composer {
        fn default() -> Self {
            let row = || gtk::Box::new(gtk::Orientation::Horizontal, 14);
            let blob_lookup: BlobLookup = Rc::new(RefCell::new(None));
            Self {
                heading: gtk::Label::new(None),
                status: gtk::Label::new(Some(UNSAVED)),
                to: gtk::Entry::new(),
                size_limit: Cell::new(None),
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
                    editor::Editor::new(std::rc::Rc::new(move |content_id: &str| {
                        lookup
                            .borrow()
                            .as_ref()
                            .and_then(|lookup| lookup(content_id))
                    }))
                },
                format_toggles: std::array::from_fn(|_| gtk::ToggleButton::new()),
                link_button: gtk::Button::new(),
                image_button: gtk::Button::new(),
                attach_button: gtk::Button::new(),
                send: gtk::Button::new(),
                schedule_send: gtk::MenuButton::new(),
                save: gtk::Button::new(),
                escape: gtk::Label::new(None),
                attach_hint: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                warning: gtk::Label::new(None),
                tracking_notice: gtk::Label::new(None),
                attachments_box: gtk::Box::new(gtk::Orientation::Vertical, 6),
                below_body: gtk::Box::new(gtk::Orientation::Vertical, 0),
                attachments_list: gtk::ListBox::new(),
                downloading: RefCell::new(std::collections::BTreeSet::new()),
                draft: RefCell::new(Draft::new(AccountId::UNASSIGNED)),
                composing_from: Cell::new(AccountId::UNASSIGNED),
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
                host: RefCell::new(None),
                previous_generation: Cell::new(0),
                generation: Cell::new(0),
                filling: Cell::new(false),
                held: RefCell::default(),
                began_as: RefCell::new(None),
                autosave_source: Cell::new(None),
                to_completion: RefCell::new(None),
                fields: gtk::Box::new(gtk::Orientation::Vertical, 0),
                chrome: RefCell::default(),
                chips: RefCell::default(),
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
    pub fn new() -> Self {
        Self::default()
    }

    /// Puts `draft` in the composer and takes over the reading pane. Instant: widgets already built
    /// are shown, so the list's
    /// scroll offset and selection are untouched.
    pub fn warm(&self) {
        self.imp().body.warm();
    }

    pub fn is_warm(&self) -> bool {
        self.imp().body.is_warm()
    }

    pub fn open(&self, draft: Draft) {
        // Already composing: a second `c` returns the keyboard (detached: raises that window),
        // never resets what is being typed.
        if self.is_open() {
            if let Some(host) = self.detached_window() {
                host.present();
            }
            self.focus_first();
            return;
        }

        // Decided before anything is filled, so the borrow is dropped by the time `fill` wants the
        // cell mutably.
        let decision = opening(&self.imp().draft.borrow(), &draft);
        if decision == Opening::Restore {
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

    /// Replaces whatever the composer holds; unlike [`open`](Self::open) it may, because a retained
    /// draft is autosaved into Drafts.
    /// Pending autosave is flushed first: the debounce timer would otherwise fire against the draft
    /// that replaced it and write
    /// one draft's words onto another's row.
    pub fn resume(&self, draft: Draft) {
        // `is_assigned`, not just id equality: every unsaved draft carries `DraftId::UNASSIGNED`,
        // so two different unsaved
        // drafts compared equal and the swap was silently declined (#1240).
        if self.is_open() && draft.id.is_assigned() && self.imp().draft.borrow().id == draft.id {
            if let Some(host) = self.detached_window() {
                host.present();
            }
            self.focus_first();
            return;
        }
        // Before `fill`, which cancels the timer: a pending edit belongs to the draft on its way
        // out.
        if self.is_open() {
            self.save();
        }
        let status = postio_ui::compose::resumed_status(&draft);
        self.fill(draft);
        self.set_status(&status);
        self.take_pane();
        self.set_visible(true);
        let continuing = starting_field(&self.imp().draft.borrow()) == Field::Body;
        if continuing {
            self.imp().body.place_caret_end();
        }
        self.focus_first();

        for handler in self.imp().opened.borrow().iter() {
            handler();
        }
    }

    pub fn is_open(&self) -> bool {
        self.is_visible()
    }

    pub fn focused_field(&self) -> Option<Field> {
        let window = self.root().and_downcast::<gtk::Window>()?;
        let focus = gtk::prelude::GtkWindowExt::focus(&window)?;
        let imp = self.imp();
        // An entry hands the keyboard to the `GtkText` inside it, so the focused widget is a
        // descendant of the field.
        if holds(&focus, imp.to.upcast_ref()) {
            Some(Field::To)
        } else if holds(&focus, imp.cc.upcast_ref()) {
            Some(Field::Cc)
        } else if holds(&focus, imp.bcc.upcast_ref()) {
            Some(Field::Bcc)
        } else if holds(&focus, imp.subject.upcast_ref()) {
            Some(Field::Subject)
        } else if holds(&focus, imp.body.widget().upcast_ref()) {
            Some(Field::Body)
        } else {
            None
        }
    }

    /// Never destroys typed content: that is [`Composer::discard`], which asks first.
    pub fn close(&self) -> Closing {
        // The pending autosave belongs to the composition being closed: run after the refill, it
        // saved the empty draft (#1608).
        self.flush_autosave();
        let draft = self.draft();
        let outcome = self.closing_now();
        if outcome == Closing::Drop {
            self.fill(Draft::new(draft.account_id));
        } else {
            *self.imp().draft.borrow_mut() = draft;
        }

        self.shut(outcome);
        outcome
    }

    fn shut(&self, outcome: Closing) {
        // Flush a pending debounced save so an edit just before closing is saved like an earlier
        // one.
        self.flush_autosave();
        // Reparented before hiding, never after: a widget with no parent has no root, and
        // `focused_field` and the pane
        // restore below both ask the root.
        self.reclaim();
        self.set_visible(false);
        self.release_pane();
        for handler in self.imp().closed.borrow().iter() {
            handler(outcome);
        }
    }

    pub fn discard(&self) {
        // The configured account, not the discarded draft's, so a discarded reply leaves nothing
        // for the next `c`.
        let account = self.account();
        self.fill(Draft::new(account));
        self.close();
    }

    /// `discard_draft` is the one composer verb marked [`Recovery::Confirm`][r]: typed prose exists
    /// nowhere else. An empty draft is discarded without asking.
    ///
    /// [r]: postio_core::Recovery
    pub fn request_discard(&self) {
        if self.closing_now() == Closing::Drop {
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

    pub fn draft(&self) -> Draft {
        let imp = self.imp();
        let mut draft = imp.draft.borrow().clone();
        match imp.chips.borrow().as_ref() {
            Some([to, cc, bcc]) => {
                draft.to = to.addresses();
                draft.cc = cc.addresses();
                draft.bcc = bcc.addresses();
            }
            None => {
                draft.to = parse_list(&imp.to.text());
                draft.cc = parse_list(&imp.cc.text());
                draft.bcc = parse_list(&imp.bcc.text());
            }
        }
        draft.subject = imp.subject.text().to_string();
        draft.body = self.body();
        // This composer writes HTML, so leftover Markdown from the terminal describes a message
        // that no longer exists.
        draft.body_markdown = None;
        draft
    }

    /// `text` is `format=flowed` (RFC 3676, #333), wrapped once here because this `MessageBody` is
    /// both filed and sent.
    /// `html` is regenerated, never carried over, and only when the document has structure
    /// `to_text` would lose; a
    /// plain-text message is sent as plain text.
    fn body(&self) -> MessageBody {
        let document = self.document();
        // Rendering `html` costs a `to_html` and a `harden` pass, so a plain-text draft must not
        // pay for it.
        let text = document.to_flowed_text();
        MessageBody {
            text: (!text.is_empty()).then_some(text),
            html: (!document.is_plain_text()).then(|| postio_body::render(&document).1),
        }
    }

    fn body_edited(&self) {
        // Programmatic fills flow through here too, and filling is not an edit.
        if self.imp().filling.get() {
            return;
        }
        self.refresh();
    }

    /// Swallowed at the floor: Ctrl+Z at the start of the history must not fall through to
    /// something else in the window.
    fn undo_edit(&self) -> glib::Propagation {
        self.imp().body.undo();
        self.refresh();
        glib::Propagation::Stop
    }

    fn redo_edit(&self) -> glib::Propagation {
        self.imp().body.redo();
        self.refresh();
        glib::Propagation::Stop
    }

    /// The Editor's record; the surface is a copy of it (ADR 0004).
    pub fn document(&self) -> postio_body::Document {
        let mut document = self.imp().body.document();
        // What is folded away is still the message: it is sent, saved and measured with the rest.
        document
            .blocks
            .extend(self.imp().held.borrow().iter().cloned());
        document
    }

    /// Folds the forwarded thread into one row a message and returns the rows; empty when the body
    /// forwards no thread.
    pub fn fold_forwarded(&self) -> Vec<postio_body::forward::ForwardedRow> {
        let imp = self.imp();
        if imp.held.borrow().is_empty() {
            let mut document = imp.body.document();
            let Some(start) = postio_body::forward::thread_start(&document) else {
                return Vec::new();
            };
            let held = document.blocks.split_off(start);
            imp.held.replace(held);
            let was = imp.filling.replace(true);
            imp.body.load(document);
            imp.filling.set(was);
        }
        postio_body::forward::thread_rows(&imp.held.borrow())
    }

    pub fn below_body(&self) -> gtk::Box {
        self.imp().below_body.clone()
    }

    pub fn unfold_forwarded(&self) {
        let imp = self.imp();
        let held = imp.held.take();
        if held.is_empty() {
            return;
        }
        let mut document = imp.body.document();
        document.blocks.extend(held);
        let was = imp.filling.replace(true);
        imp.body.load(document);
        imp.filling.set(was);
    }

    pub fn identity(&self) -> Option<Identity> {
        let selected = self.imp().identity.selected() as usize;
        self.imp().identities.borrow().get(selected).cloned()
    }

    /// One identity is not a choice, so it is drawn as text, not a picker. The default's signature
    /// goes into the body at once.
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
        self.apply_signature();
    }

    /// The override is the draft's: recorded on it, so a later reply picks its own identity again.
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
        self.apply_signature();
        true
    }

    /// `None` when no limit is configured, and then nothing is checked: a guessed ceiling refuses
    /// mail the provider would take.
    pub fn set_size_limit(&self, limit: Option<u64>) {
        self.imp().size_limit.set(limit);
    }

    /// Checked here because a server's rejection arrives after the composer has closed.
    fn too_large(&self) -> Option<postio_model::size::TooLarge> {
        postio_model::size::check(&self.draft(), self.imp().size_limit.get())
    }

    /// Hidden when the account has none: a one-entry picker only says what is already true.
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
        self.apply_signature();
    }

    pub fn has_signature(&self, id: SignatureId) -> bool {
        self.imp()
            .signatures
            .borrow()
            .iter()
            .any(|signature| signature.id == id)
    }

    /// `false` when the account lacks it; the caller falls back to the identity's own rather than
    /// leave a stale selection.
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
        // Entry 0 is the identity's own; the account's signatures follow.
        self.imp().signature.set_selected((index + 1) as u32);
        true
    }

    /// What `c` starts. Resolves the signature through
    /// [`connect_signature_default`](Self::connect_signature_default), so
    /// "neither has an opinion" resets the picker to the identity's own rather than an older
    /// draft's choice (#394).
    fn open_new_draft(&self) {
        self.open(Draft::new(self.account()));
        let generation = self.generation();
        let weak = self.downgrade();
        let answer: SignatureAnswer = Box::new(move |resolved| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
            // For the composition it was asked for only: a late answer names nothing here.
            if composer.generation() != generation {
                return;
            }
            let selected = resolved.is_some_and(|id| composer.select_signature(id));
            if !selected {
                composer.imp().signature.set_selected(0);
            }
        });
        let provider = self.imp().signature_default.borrow();
        match provider.as_ref() {
            Some(provider) => provider(answer),
            None => answer(None),
        }
    }

    fn chosen_signature(&self, identity: &Identity) -> Option<Signature> {
        let imp = self.imp();
        // An empty picker reports `INVALID_LIST_POSITION`, not zero, so entry 0 and "no entries"
        // both mean the identity's own.
        let chosen = match imp.signature.selected() {
            gtk::INVALID_LIST_POSITION => None,
            selected => (selected as usize).checked_sub(1),
        };
        match chosen {
            Some(index) => imp.signatures.borrow().get(index).cloned(),
            None => identity.signature.clone(),
        }
    }

    pub fn set_signature_placement(&self, on_reply: Placement, on_forward: Placement) {
        self.imp().signature_placement.set((on_reply, on_forward));
    }

    fn signature_placement(&self) -> Placement {
        let (on_reply, on_forward) = self.imp().signature_placement.get();
        match self.imp().draft.borrow().kind {
            DraftKind::Reply | DraftKind::ReplyAll => on_reply,
            DraftKind::Forward => on_forward,
            DraftKind::New => Placement::BelowQuote,
        }
    }

    /// Does not touch the body: changing who a draft is from must not rewrite a signature the user
    /// edited, and
    /// `postio_body::apply_signature` matches the separator, not intent. The signature picker owns
    /// the body half.
    fn apply_identity(&self) {
        let Some(identity) = self.identity() else {
            return;
        };
        let imp = self.imp();
        let mut draft = self.draft();
        draft.use_identity(&identity);
        imp.draft.borrow_mut().identity_id = draft.identity_id;
    }

    /// Run when a draft opens, where nothing is typed yet, and when the picker is used; never on an
    /// identity change.
    fn apply_signature(&self) {
        let Some(identity) = self.identity() else {
            return;
        };
        let imp = self.imp();

        // At the block level for every draft, never through text: flattening a rich quote to `> `
        // lines would let the
        // identity dropdown destroy the reply (#340).
        let current = imp.body.document();
        let signature = self.chosen_signature(&identity);
        let wanted =
            postio_body::apply_signature(&current, signature.as_ref(), self.signature_placement());
        if current == wanted {
            return;
        }

        // The caret goes to the start of the body: preserving the exact offset across an Editor
        // reload needs a script round trip.
        let was_filling = imp.filling.replace(true);
        imp.body.load(wanted);
        imp.body.place_caret_start();
        imp.filling.set(was_filling);
        self.refresh();
    }

    pub fn status(&self) -> String {
        self.imp().status.text().to_string()
    }

    /// Set by whoever did it: the composer never claims a draft was saved on its own behalf.
    pub fn set_status(&self, status: &str) {
        self.imp().status.set_text(status);
    }

    /// With nothing connected the draft stays and the status line names what is missing. The key
    /// checks what the Send button
    /// already checks ([`Draft::is_sendable`]): with no recipients the composer would clear and the
    /// queued operation drain
    /// as impossible, and an already-queued draft resumed from Drafts would send twice.
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
        if let Some(refusal) = self.too_large() {
            self.set_status(&refusal.to_string());
            return;
        }
        // Asked rather than refused: a message can have no subject. One question for however many
        // concerns apply.
        let concerns = send_concerns(&draft);
        if !concerns.is_empty() {
            self.ask_before_sending(&concerns);
            return;
        }
        self.hand_off(draft);
    }

    fn hand_off(&self, draft: Draft) {
        for handler in self.imp().sent.borrow().iter() {
            handler(&draft);
        }
        let account = draft.account_id;
        self.fill(Draft::new(account));
        self.shut(Closing::Drop);
    }

    /// One dialog for however many concerns: a second question in a row gets answered without
    /// reading.
    fn ask_before_sending(&self, concerns: &[String]) {
        let body = format!("{}. Esc keeps the composer open.", join_with_and(concerns));
        let dialog = adw::AlertDialog::new(Some("Send it anyway?"), Some(&body));
        dialog.add_responses(&[("back", "Go back"), ("send", "Send anyway")]);
        dialog.set_default_response(Some("back"));
        dialog.set_close_response("back");
        dialog.connect_response(
            None,
            glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |_, response| {
                    if response == "send" {
                        let draft = composer.draft();
                        composer.hand_off(draft);
                    }
                }
            ),
        );
        dialog.present(Some(self));
    }

    /// The same sendability checks as [`send`](Self::send).
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
        // Refusing at the scheduled hour, when nobody is watching, is worse than refusing now.
        if let Some(refusal) = self.too_large() {
            self.set_status(&refusal.to_string());
            return;
        }
        for handler in self.imp().sent_later.borrow().iter() {
            handler(&draft, when);
        }
        let account = draft.account_id;
        self.fill(Draft::new(account));
        self.shut(Closing::Drop);
    }

    /// A handler may assign the draft an id; it is written back so the next save updates the same
    /// row. Nothing else a handler touches is kept.
    pub fn save(&self) {
        let mut draft = self.draft();
        // Nothing written and never saved: not worth a row. The editor reports asynchronously, so
        // an edit can re-arm the
        // autosave after a close refilled the fields, and that timer inserted an empty `Editing`
        // row (#1608).
        if !draft.id.is_assigned() && self.closing_now() == Closing::Drop {
            return;
        }
        for handler in self.imp().saved.borrow().iter() {
            handler(&mut draft);
        }
        self.imp().draft.borrow_mut().id = draft.id;
    }

    pub fn generation(&self) -> u64 {
        self.imp().generation.get()
    }

    pub fn previous_generation(&self) -> u64 {
        self.imp().previous_generation.get()
    }

    /// Saves are written off the main thread, so the id arrives after [`Composer::save`] returned.
    /// It is taken only while
    /// the same composition is in the fields and has no id yet, so another draft's save never
    /// updates this row.
    pub fn adopt_id(&self, generation: u64, id: postio_model::ids::DraftId) {
        if self.generation() != generation {
            return;
        }
        let mut draft = self.imp().draft.borrow_mut();
        if !draft.id.is_assigned() {
            draft.id = id;
        }
    }

    pub fn connect_send(&self, handler: impl Fn(&Draft) + 'static) {
        self.imp().sent.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_send_later(&self, handler: impl Fn(&Draft, DateTime<Utc>) + 'static) {
        self.imp().sent_later.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_save(&self, handler: impl Fn(&mut Draft) + 'static) {
        self.imp().saved.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_changed(&self, handler: impl Fn(&Draft) + 'static) {
        self.imp().changed.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_closed(&self, handler: impl Fn(Closing) + 'static) {
        self.imp().closed.borrow_mut().push(Box::new(handler));
    }

    /// Not called when `c` finds the composer already open.
    pub fn connect_opened(&self, handler: impl Fn() + 'static) {
        self.imp().opened.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_reply_source(&self, provider: impl Fn(ReplyAnswer) + 'static) {
        *self.imp().reply_source.borrow_mut() = Some(Box::new(provider));
    }

    /// Called once, when `c` starts a draft, so it answers for the mailbox selected right then.
    /// `None` leaves the picker on the identity's own.
    pub fn connect_signature_default(&self, provider: impl Fn(SignatureAnswer) + 'static) {
        *self.imp().signature_default.borrow_mut() = Some(Box::new(provider));
    }

    /// The composer only shows what comes back, in that order, and never touches the network.
    pub fn connect_recipient_suggestions(
        &self,
        provider: impl Fn(&str) -> Vec<RecipientCandidate> + 'static,
    ) {
        *self.imp().recipient_suggestions.borrow_mut() = Some(Box::new(provider));
    }

    /// `handler` may call the callback at once or hand it to a background task; `add_file` does not
    /// block on it.
    pub fn connect_attach(&self, handler: impl Fn(std::path::PathBuf, AttachReady) + 'static) {
        *self.imp().attach.borrow_mut() = Some(Box::new(handler));
    }

    pub fn connect_inline_image(
        &self,
        handler: impl Fn(Vec<u8>, String, InlineImageReady) + 'static,
    ) {
        *self.imp().inline_image.borrow_mut() = Some(Box::new(handler));
    }

    pub fn connect_attachment_bytes(
        &self,
        reader: impl Fn(&Attachment) -> Option<Vec<u8>> + 'static,
    ) {
        *self.imp().attachment_bytes.borrow_mut() = Some(Box::new(reader));
    }

    pub fn is_detached(&self) -> bool {
        self.imp().detached.borrow().is_some()
    }

    pub fn detached_window(&self) -> Option<adw::Window> {
        self.imp().detached.borrow().clone()
    }

    /// A no-op when nothing is being composed: an empty composer to detach would be a second way to
    /// start writing.
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

    /// The same widget, reparented, so every field, the identity override, the undo history and the
    /// cursor survive without being copied.
    pub fn detach(&self) {
        if self.is_detached() || !self.is_open() {
            return;
        }
        let Some(holder) = self.host() else {
            return;
        };
        let field = self.focused_field();

        // The main window goes all the way back first, so the two windows are independent.
        self.release_pane();

        let host = adw::Window::builder()
            .title(self.window_title())
            .transient_for(&holder.parent().unwrap_or_default())
            // Not modal: the point of detaching is to read something else while writing.
            .modal(false)
            .default_width(DETACHED_SIZE.0)
            .default_height(DETACHED_SIZE.1)
            .build();

        // A real header bar: `AdwWindow` draws no titlebar, so a bare `set_content(composer)` would
        // have no title, close button or drag handle.
        holder.remove(self);
        match holder.frame() {
            // The host's frame carries its own header, so the window is the frame; the handle keeps
            // it draggable.
            Some(frame) => {
                let handle = gtk::WindowHandle::new();
                handle.set_child(Some(&frame));
                host.set_content(Some(&handle));
            }
            None => {
                let layout = adw::ToolbarView::new();
                layout.add_top_bar(&adw::HeaderBar::new());
                layout.set_content(Some(self));
                host.set_content(Some(&layout));
            }
        }
        self.set_visible(true);

        // A forwarder to the main window's resolver, so `[keys]` reaches both containers with one
        // keymap.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| composer.handle_key(key, state)
        ));
        host.add_controller(keys);

        // The close button means what `Esc` means: keep the draft. Discarding stays explicit and
        // asks first.
        host.connect_close_request(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_| {
                composer.close();
                // `close` already destroyed the window; the default handler would tear down a
                // window that is gone.
                glib::Propagation::Stop
            }
        ));

        self.imp().detached.replace(Some(host.clone()));
        sync_detach_button(&self.imp().detach, true);
        host.present();
        self.restore_focus(field);
    }

    /// The subject, so a detached window is identifiable without being focused; the kind is the
    /// fallback when there is no subject.
    fn window_title(&self) -> String {
        let draft = self.imp().draft.borrow();
        let subject = draft.subject.trim();
        if subject.is_empty() {
            heading(draft.kind).to_owned()
        } else {
            subject.to_owned()
        }
    }

    pub fn attach(&self) {
        let field = self.focused_field();
        if !self.reclaim() {
            return;
        }
        self.take_pane();
        self.set_visible(true);
        self.restore_focus(field);
    }

    /// Unparenting a widget drops the focus; falls back to the field a fresh composition would
    /// start in.
    fn restore_focus(&self, field: Option<Field>) {
        match field {
            Some(field) => {
                self.widget_for(field).grab_focus();
            }
            None => self.focus_first(),
        }
    }

    /// `false` when it was not detached. Does not take the pane over again, which
    /// [`attach`](Self::attach) does and closing must not.
    fn reclaim(&self) -> bool {
        let Some(host) = self.imp().detached.take() else {
            return false;
        };
        // Out of the toolbar view, not off the window: unparenting it from anywhere else would
        // leave GTK holding a missing child.
        if let Some(layout) = host.content().and_downcast::<adw::ToolbarView>() {
            layout.set_content(None::<&gtk::Widget>);
        }
        if let Some(handle) = host.content().and_downcast::<gtk::WindowHandle>() {
            handle.set_child(None::<&gtk::Widget>);
        }
        host.set_content(None::<&gtk::Widget>);
        if let Some(holder) = self.host() {
            holder.restore(self);
        }
        // `destroy`, not `close`: `close` emits `close-request`, and this is reached from inside
        // that handler.
        host.destroy();
        sync_detach_button(&self.imp().detach, false);
        true
    }

    /// Public so a test can press keys: GTK4 has no supported way to synthesize a GDK event.
    pub fn handle_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        let Some(holder) = self.host() else {
            return glib::Propagation::Proceed;
        };
        let Some(host) = self.detached_window() else {
            return glib::Propagation::Proceed;
        };
        let host: gtk::Window = host.upcast();
        // Resolved by the host, dispatched by this composer: the host's own key handling broadcasts
        // to every subscriber, so
        // with several composers open `Send` would send every draft (ADR 0034).
        match holder.command_for_key(key, state, &host) {
            Some(id) => {
                self.dispatch(id);
                glib::Propagation::Stop
            }
            None => holder.handle_key(key, state, &host),
        }
    }

    pub fn mount_on(&self, host: Rc<dyn ComposerHost>) {
        self.imp().host.replace(Some(Rc::clone(&host)));
        host.install(self);

        let action = gio::SimpleAction::new("compose", None);
        action.connect_activate(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_, _| {
                // The header button reaches this whether or not the composer is open; only the
                // button doubles as the close.
                // The keyboard's `c` reaches `dispatch` and stays a pure open.
                if let Some(host) = composer.detached_window() {
                    // Detached, the useful thing is to put the other window in front.
                    host.present();
                } else if composer.is_open() {
                    composer.close();
                } else {
                    composer.open_new_draft();
                }
            }
        ));
        host.add_action(&action);

        // The broadcast belongs to whichever composer has the pane: a detached one hears its own
        // keys, and with two open
        // `Send` would otherwise send both drafts (ADR 0034).
        host.connect_command(Box::new(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |id| {
                if composer.is_detached() {
                    return;
                }
                composer.dispatch(id);
            }
        )));
    }

    /// A host that keeps the composer alive and is kept alive by it is a reference cycle; whoever
    /// owns the host calls this on teardown.
    pub fn unmount(&self) {
        self.imp().host.take();
    }

    fn host(&self) -> Option<Rc<dyn ComposerHost>> {
        self.imp().host.borrow().clone()
    }

    pub fn dispatch(&self, id: CommandId) {
        match id {
            CommandId::Compose => self.open_new_draft(),
            CommandId::Send if self.is_open() => self.send(),
            CommandId::ScheduleSend if self.is_open() => self.imp().schedule_send.popup(),
            CommandId::SaveDraft if self.is_open() => self.save(),
            CommandId::DiscardDraft if self.is_open() => self.request_discard(),
            CommandId::AttachFile if self.is_open() => self.open_file_chooser(),
            CommandId::InsertImage if self.is_open() => self.open_image_chooser(),
            CommandId::DetachComposer if self.is_open() => self.toggle_detached(),
            CommandId::CopyFields if self.is_open() => self.toggle_copy_fields(),
            CommandId::Back if self.is_open() => {
                self.close();
            }
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward if !self.is_open() => {
                self.open_reply(id);
            }
            // Replacing an in-progress draft with a fresh reply would lose typed prose without
            // confirmation, so refuse and say so on the status line (#426).
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward => {
                self.set_status(REPLY_BLOCKED);
            }
            CommandId::Bold if self.is_open() => {
                self.imp().body.format(editor::Format::Bold);
            }
            CommandId::Italic if self.is_open() => {
                self.imp().body.format(editor::Format::Italic);
            }
            CommandId::Heading1 if self.is_open() => {
                self.imp().body.format(editor::Format::Heading1);
            }
            CommandId::BulletList if self.is_open() => {
                self.imp().body.format(editor::Format::BulletList);
            }
            CommandId::NumberedList if self.is_open() => {
                self.imp().body.format(editor::Format::NumberedList);
            }
            CommandId::QuoteBlock if self.is_open() => {
                self.imp().body.format(editor::Format::QuoteBlock);
            }
            CommandId::InsertLink if self.is_open() => self.request_link(),
            _ => {}
        }
    }

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

    /// Silently does nothing without a source or when it has nothing to offer: `e` with no message
    /// open is not an error.
    fn open_reply(&self, id: CommandId) {
        let weak = self.downgrade();
        let answer: ReplyAnswer = Box::new(move |found| {
            if let Some(composer) = weak.upgrade() {
                composer.reply_with(id, found);
            }
        });
        let provider = self.imp().reply_source.borrow();
        if let Some(provider) = provider.as_ref() {
            provider(answer);
        }
    }

    fn reply_with(&self, id: CommandId, found: Option<(Message, Account)>) {
        let Some((source, account)) = found else {
            return;
        };
        // The picker must speak for this account before `open` fills the draft: `fill` resolves
        // `identity_id` against
        // `imp.identities`, which was last set for the window's default account, so a reply to
        // account B sent as A (#189).
        self.set_identities(account.identities.clone());
        self.set_signatures(account.signatures.clone());
        if let Some(draft) = reply_draft(id, &source, &account) {
            // After, not before: `open` calls `fill`, which clears the notice, and the reply's own
            // domains must win.
            self.open(draft);
            self.set_tracking_domains(&quoted_tracking_domains(&source));
        }
    }

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

    /// The one `set_account` named, else the draft cell's. Not the cell's first: a reply fills the
    /// cell with the answered message's account.
    fn account(&self) -> AccountId {
        let configured = self.imp().composing_from.get();
        if configured == AccountId::UNASSIGNED {
            self.imp().draft.borrow().account_id
        } else {
            configured
        }
    }

    /// Meant to be called once before the first composition: it writes onto the current draft.
    pub fn set_account(&self, account_id: AccountId) {
        self.imp().composing_from.set(account_id);
        self.imp().draft.borrow_mut().account_id = account_id;
    }

    /// Opt-in for the composer's life: the fields are otherwise plain text.
    pub fn set_recipient_chips(&self, on: bool) {
        let imp = self.imp();
        if !on || imp.chips.borrow().is_some() {
            return;
        }
        let chips = [&imp.to, &imp.cc, &imp.bcc].map(|entry| {
            let chips = crate::widgets::recipients::RecipientChips::around(entry);
            chips.connect_changed(glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move || composer.refresh()
            ));
            chips
        });
        imp.chips.replace(Some(chips));
    }

    /// Hides the heading row, action row, paperclip and "Attach another" hint, for a host whose
    /// frame draws them.
    pub fn set_framed(&self, framed: bool) {
        let imp = self.imp();
        for widget in imp.chrome.borrow().iter() {
            widget.set_visible(!framed);
        }
        imp.attach_button.set_visible(!framed);
        imp.attach_hint.set_visible(!framed);
    }

    /// Each colour is read from a probe wearing a `.postio-flow-*` class whose `color` the host's
    /// stylesheet sets, so no colour is written in Rust.
    pub fn flow_in_column(&self) {
        let Some(column) = self.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let mut probes = Vec::new();
        for (variable, class) in std::iter::once((editor::GROUND, "postio-flow-ground"))
            .chain(crate::reader::view::FLOW_PALETTE)
        {
            let probe = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            probe.add_css_class(class);
            probe.set_accessible_role(gtk::AccessibleRole::Presentation);
            column.append(&probe);
            probes.push((variable, probe.upcast::<gtk::Widget>()));
        }
        self.imp().body.flow_in(probes);
    }

    pub fn add_field_row(&self, row: &impl IsA<gtk::Widget>) {
        self.imp().fields.append(row);
    }

    pub fn labels(&self) -> Vec<postio_model::LabelId> {
        self.imp().draft.borrow().labels.clone()
    }

    pub fn set_labels(&self, labels: Vec<postio_model::LabelId>) {
        self.imp().draft.borrow_mut().labels = labels;
        self.refresh();
    }

    pub fn remind_at(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        self.imp().draft.borrow().remind_at
    }

    pub fn set_remind_at(&self, at: Option<chrono::DateTime<chrono::Utc>>) {
        self.imp().draft.borrow_mut().remind_at = at;
        self.refresh();
    }

    /// Part of what the composition opened with, not an edit: nothing is saved until the person
    /// writes something.
    pub fn start_with_labels(&self, labels: Vec<postio_model::LabelId>) {
        let imp = self.imp();
        imp.draft.borrow_mut().labels = labels;
        let was_filling = imp.filling.replace(true);
        self.refresh();
        imp.filling.set(was_filling);
    }

    /// A picture chosen here goes inside the message at the caret, unlike
    /// [`Self::open_file_chooser`], where it lands beside it.
    /// `GtkFileDialog` goes through the XDG portal, so this works under Flatpak.
    fn open_image_chooser(&self) {
        let Some(window) = self.host().and_then(|host| host.parent()) else {
            return;
        };
        // Unnamed on purpose: `check-uncalled-pub-fn` matches by bare name, and calling `set_name`
        // here would mask an unrelated one.
        let filter = gtk::FileFilter::new();
        filter.add_mime_type("image/*");
        let filters = gio::ListStore::new::<gtk::FileFilter>();
        filters.append(&filter);

        let dialog = gtk::FileDialog::builder()
            .title("Insert image")
            .filters(&filters)
            .build();
        dialog.open(
            Some(&window),
            gio::Cancellable::NONE,
            glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |result| {
                    let Ok(file) = result else {
                        return;
                    };
                    composer.insert_image_file(&file);
                }
            ),
        );
    }

    fn insert_image_file(&self, file: &gio::File) {
        let bytes = match file.load_contents(gio::Cancellable::NONE) {
            Ok((bytes, _)) => bytes,
            Err(error) => {
                self.set_status(&format!("That image could not be read: {error}"));
                return;
            }
        };
        // From the content, not the extension: a `.png` that is a JPEG would reach the recipient
        // declared wrongly.
        let mime = gio::content_type_guess(file.basename().as_deref(), Some(bytes.as_ref()))
            .0
            .to_string();
        if !mime.starts_with("image/") {
            self.set_status("That file is not an image. Use Attach file to send it alongside.");
            return;
        }
        self.add_inline_image(bytes.to_vec(), &mime);
    }

    fn open_file_chooser(&self) {
        let Some(window) = self.host().and_then(|host| host.parent()) else {
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

    /// Counts as an edit, so autosave and `changed` handlers see it.
    fn add_attachment(&self, attachment: Attachment) {
        self.imp().draft.borrow_mut().attachments.push(attachment);
        self.render_attachments();
        self.refresh();
    }

    /// Text falls through to WebKit's own paste. Returns whether the paste was taken.
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

    fn remove_attachment(&self, index: usize) {
        let imp = self.imp();
        if index < imp.draft.borrow().attachments.len() {
            imp.draft.borrow_mut().attachments.remove(index);
        }
        self.render_attachments();
        self.refresh();
    }

    /// Empty says nothing is on its way.
    pub fn set_downloading(&self, parts: impl IntoIterator<Item = String>) {
        let parts: std::collections::BTreeSet<String> = parts.into_iter().collect();
        if *self.imp().downloading.borrow() == parts {
            return;
        }
        self.imp().downloading.replace(parts);
        self.render_attachments();
    }

    /// A refused part is fetched again when the forward is sent, so it reads as an ordinary
    /// attachment.
    pub fn fetch_settled(
        &self,
        message: postio_model::MessageId,
        sections: &postio_core::event::Sections,
    ) {
        let imp = self.imp();
        if imp.draft.borrow().forwarded_from != Some(message) {
            return;
        }
        let before = imp.downloading.borrow().len();
        imp.downloading
            .borrow_mut()
            .retain(|part| !sections.holds_part(part));
        if imp.downloading.borrow().len() != before {
            self.render_attachments();
        }
    }

    /// The draft's list is the source of truth, never the widgets.
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

    fn build_attachment_row(&self, index: usize, attachment: &Attachment) -> gtk::Widget {
        let downloading = attachment
            .part_id
            .as_ref()
            .is_some_and(|part| self.imp().downloading.borrow().contains(part));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.update_property(&[gtk::accessible::Property::Label(&if downloading {
            format!("Attachment: {}, downloading", attachment.display_name())
        } else {
            format!("Attachment: {}", attachment.display_name())
        })]);

        let name = gtk::Label::new(Some(attachment.display_name()));
        name.set_xalign(0.0);
        name.set_hexpand(true);
        name.set_ellipsize(pango::EllipsizeMode::Middle);

        let meta_text = postio_ui::compose::attachment_note(
            &postio_ui::format::human_size(attachment.size),
            attachment.size >= LARGE_ATTACHMENT_BYTES,
            downloading,
        );
        let meta = gtk::Label::new(Some(&meta_text));
        meta.add_css_class("postio-compose-label");
        meta.add_css_class("dim-label");

        let remove = crate::widgets::icon_button(
            "edit-delete-symbolic",
            &format!("Remove {}", attachment.display_name()),
        );
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

    fn build_attachments(&self) -> gtk::Box {
        let imp = self.imp();
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some("Attachments"));
        title.add_css_class("postio-compose-label");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        header.append(&title);
        header.append(&imp.attach_hint);

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

    fn take_pane(&self) {
        if let Some(host) = self.host() {
            host.take_pane();
        }
    }

    fn release_pane(&self) {
        if let Some(host) = self.host() {
            host.release_pane();
        }
    }

    fn fill(&self, draft: Draft) {
        let imp = self.imp();
        imp.previous_generation.set(imp.generation.get());
        imp.generation.set(imp.generation.get() + 1);
        imp.filling.set(true);
        // Filling is not an edit worth autosaving, and a timer armed for the previous content must
        // not fire against this one.
        self.cancel_autosave();

        let chips = imp.chips.borrow().clone();
        match chips {
            Some([to, cc, bcc]) => {
                to.set_addresses(&draft.to);
                cc.set_addresses(&draft.cc);
                bcc.set_addresses(&draft.bcc);
            }
            None => {
                imp.to.set_text(&format_list(&draft.to));
                imp.cc.set_text(&format_list(&draft.cc));
                imp.bcc.set_text(&format_list(&draft.bcc));
            }
        }
        imp.subject.set_text(&draft.subject);
        // The document first, then the view over it: an HTML body is parsed, which makes a reply to
        // HTML-only mail editable.
        let document = document_of(&draft.body);
        // `load` clears the Editor's history, so undo cannot put one message's words into
        // another's.
        imp.held.take();
        imp.body.load(document);
        imp.heading.set_text(heading(draft.kind));

        imp.cc_row.set_visible(!draft.cc.is_empty());
        imp.bcc_row.set_visible(!draft.bcc.is_empty());
        self.sync_more();
        // Ephemeral to one reply action: `open_reply` sets it right after this call returns.
        self.set_tracking_domains(&[]);

        let identity = draft.identity_id;
        *imp.draft.borrow_mut() = draft;
        imp.downloading.borrow_mut().clear();
        imp.filling.set(false);
        self.render_attachments();

        // The draft's own identity wins over the account default.
        if !identity.is_some_and(|id| self.select_identity(id)) {
            self.apply_identity();
            self.apply_signature();
        }

        imp.body.place_caret_start();
        *imp.began_as.borrow_mut() = Some(self.draft());
        self.refresh();
    }

    fn closing_now(&self) -> Closing {
        let draft = self.draft();
        match self.imp().began_as.borrow().as_ref() {
            Some(began) => closing_since(&draft, began),
            None => closing(&draft),
        }
    }

    /// A widget that is not mapped cannot take focus, and on the opening frame the composer is not:
    /// the grab is retried
    /// from a tick callback, not an idle, because `idle_add_local_once` is not ordered against the
    /// frame clock's layout
    /// pass and can lose the race (`postio-43`). Two ticks put the retry after the layout pass.
    fn focus_first(&self) {
        let imp = self.imp();
        let field: gtk::Widget = self.widget_for(starting_field(&imp.draft.borrow()));
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

    pub fn show_copy_fields(&self) {
        let imp = self.imp();
        imp.cc_row.set_visible(true);
        imp.bcc_row.set_visible(true);
        self.sync_more();
        imp.cc.grab_focus();
    }

    /// Putting away works only while both fields are empty: a hidden row that held addresses would
    /// still send to them
    /// unseen. When it will not, the keyboard goes to `Cc` instead, so the refusal is visible.
    pub fn toggle_copy_fields(&self) {
        let imp = self.imp();
        if !imp.cc_row.is_visible() || !imp.bcc_row.is_visible() {
            self.show_copy_fields();
            return;
        }
        let draft = self.draft();
        if draft.cc.is_empty() && draft.bcc.is_empty() {
            imp.cc_row.set_visible(false);
            imp.bcc_row.set_visible(false);
            self.sync_more();
            // The keyboard cannot stay in a row that is no longer on screen, or Tab resumes from
            // somewhere invisible.
            imp.to.grab_focus();
        } else {
            imp.cc.grab_focus();
        }
    }

    fn sync_more(&self) {
        let imp = self.imp();
        imp.more
            .set_visible(!(imp.cc_row.is_visible() && imp.bcc_row.is_visible()));
    }

    fn refresh(&self) {
        let imp = self.imp();
        let draft = self.draft();

        // A problem outranks a count: an address that will bounce matters more than a tally.
        match recipient_warning(&draft).or_else(|| recipient_summary(&draft)) {
            Some(text) => {
                imp.warning.set_text(&text);
                imp.warning.set_visible(true);
            }
            None => imp.warning.set_visible(false),
        }
        imp.send.set_sensitive(draft.is_sendable());

        // A detached window's title tells it apart in the window list, and the subject is usually
        // typed after detaching.
        if let Some(host) = self.detached_window() {
            let title = self.window_title();
            if host.title().as_deref() != Some(title.as_str()) {
                host.set_title(Some(&title));
            }
        }

        if imp.filling.get() {
            return;
        }
        for handler in imp.changed.borrow().iter() {
            handler(&draft);
        }
        self.schedule_autosave();
    }

    /// A source id is not `Clone`, so re-arming cancels whatever was pending first.
    fn schedule_autosave(&self) {
        // Not mounted yet: this is `build()`'s initial layout, not a user edit, and a save has
        // nowhere to go.
        if self.host().is_none() {
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

    /// For content about to stop existing (fresh draft, discard, send), where firing later would
    /// save the wrong thing.
    fn cancel_autosave(&self) {
        if let Some(source) = self.imp().autosave_source.take() {
            source.remove();
        }
    }

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
        // `Status` is a live region: a screen reader announces changes without moving focus.
        imp.status.set_accessible_role(gtk::AccessibleRole::Status);

        crate::widgets::button::style(
            &imp.detach,
            crate::widgets::button::Kind::Ghost,
            crate::widgets::button::Size::Small,
        );
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
        imp.status.set_hexpand(true);
        title.append(&imp.detach);

        let fields = &imp.fields;
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
        // The Editor owns the document, its history and the typing-run coalescing; editing undo is
        // the document's, mail undo is `u` (ADR 0004).
        imp.body.connect_changed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.body_edited()
        ));

        // Capture phase is load-bearing: WebKit keeps its own editing undo on a contenteditable
        // surface, so letting the key
        // through would run two histories against one document.
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
                    // A clipboard holding pixels is ours: WebKit would write an unresolvable fake
                    // URL into the DOM (#341).
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
        imp.body
            .widget()
            .update_property(&[gtk::accessible::Property::Label(BODY_NAME)]);

        // An image goes inline; anything else falls through to the composer-wide target as an
        // attachment.
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
        column.append(fields);
        column.append(&imp.tracking_notice);
        column.append(&self.build_toolbar());
        column.append(imp.body.widget());
        column.append(&imp.below_body);
        column.append(&imp.warning);
        column.append(&self.build_attachments());
        let actions = self.build_actions();
        column.append(&actions);
        imp.chrome
            .replace(vec![title.clone().upcast(), actions.upcast()]);
        self.set_child(Some(&column));
        self.install_drop_target();

        for entry in [&imp.to, &imp.cc, &imp.bcc, &imp.subject] {
            entry.connect_changed(glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |_| composer.refresh()
            ));
        }

        *imp.to_completion.borrow_mut() = Some(Completion::install(self, &imp.to));
        Completion::install(self, &imp.cc);
        Completion::install(self, &imp.bcc);

        self.set_identities(Vec::new());
        self.refresh();
    }

    /// Buttons keep their hands off the focus (`focus_on_click` false): stealing the caret would
    /// destroy the selection the command acts on.
    fn build_toolbar(&self) -> gtk::Box {
        let imp = self.imp();
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        toolbar.add_css_class("postio-compose-toolbar");
        toolbar.set_accessible_role(gtk::AccessibleRole::Toolbar);
        toolbar.update_property(&[gtk::accessible::Property::Label("Formatting")]);

        for (button, (id, icon, class)) in imp.format_toggles.iter().zip(FORMAT_TOGGLES) {
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

        style_toolbar_button(
            &imp.image_button,
            CommandId::InsertImage,
            "insert-image-symbolic",
            "postio-toolbar-image",
        );
        imp.image_button.set_focus_on_click(true);
        imp.image_button.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.dispatch(CommandId::InsertImage)
        ));
        toolbar.append(&imp.image_button);

        // Through `dispatch`, like every other button here, so the control and the key cannot come
        // to mean different things (#1197).
        style_toolbar_button(
            &imp.attach_button,
            CommandId::AttachFile,
            "mail-attachment-symbolic",
            "postio-toolbar-attach",
        );
        // Unlike the formatting buttons it does not act on the selection, so it takes focus and
        // stays keyboard-reachable.
        imp.attach_button.set_focus_on_click(true);
        imp.attach_button.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.dispatch(CommandId::AttachFile)
        ));
        toolbar.append(&imp.attach_button);

        // Programmatic `set_active` emits `toggled`, not `clicked`, so nothing loops.
        let toggles = imp.format_toggles.clone();
        imp.body.connect_format_state(move |state| {
            let states = [
                state.bold,
                state.italic,
                state.heading_1,
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

    fn build_to_row(&self) -> gtk::Box {
        let imp = self.imp();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 14);

        imp.more.set_label("+ Cc");
        imp.more.add_css_class("flat");
        imp.more.add_css_class("postio-compose-more");
        imp.more
            .update_property(&[gtk::accessible::Property::Label("Show Cc and Bcc")]);
        // The same verb the keyboard and the palette reach, not a second implementation to keep in
        // step.
        imp.more.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.toggle_copy_fields()
        ));

        let row = self.build_row(&row, "To", &imp.to);
        row.append(&imp.more);
        row
    }

    fn build_identity_row(&self) -> gtk::Box {
        let imp = self.imp();
        let row = &imp.identity_row;
        row.add_css_class("postio-compose-row");
        row.append(&field_label("From"));

        imp.identity.add_css_class("postio-compose-identity");
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

        // A separate control on purpose: choosing a signature must not change the sending address
        // (#12).
        imp.signature.add_css_class("postio-compose-identity");
        imp.signature.add_css_class("flat");
        imp.signature.set_halign(gtk::Align::End);
        imp.signature
            .update_property(&[gtk::accessible::Property::Label("Signature")]);
        imp.signature.connect_selected_notify(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.apply_signature()
        ));
        imp.signature.set_visible(false);
        row.append(&imp.signature);
        row.clone()
    }

    fn build_row(&self, row: &gtk::Box, label: &str, entry: &gtk::Entry) -> gtk::Box {
        row.add_css_class("postio-compose-row");
        entry.set_hexpand(true);
        entry.add_css_class("postio-compose-field");
        entry.update_property(&[gtk::accessible::Property::Label(label)]);
        row.append(&field_label(label));
        row.append(entry);
        row.clone()
    }

    /// Discard is deliberately absent: a one-click destructive button beside Send is a misclick
    /// waiting; `ctrl+d` and the
    /// palette reach it and ask first.
    fn build_actions(&self) -> gtk::Box {
        let imp = self.imp();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("postio-compose-actions");

        crate::widgets::button::style(
            &imp.send,
            crate::widgets::button::Kind::Primary,
            crate::widgets::button::Size::Regular,
        );
        imp.send
            .update_property(&[gtk::accessible::Property::Label("Send")]);
        imp.send.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.send()
        ));

        crate::widgets::button::style(
            &imp.schedule_send,
            crate::widgets::button::Kind::Ghost,
            crate::widgets::button::Size::Regular,
        );
        imp.schedule_send
            .update_property(&[gtk::accessible::Property::Label("Schedule send")]);
        // Rebuilt on every open: the presets are relative to the moment it opens.
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

        crate::widgets::button::style(
            &imp.save,
            crate::widgets::button::Kind::Ghost,
            crate::widgets::button::Size::Regular,
        );
        imp.save
            .update_property(&[gtk::accessible::Property::Label("Save draft")]);
        imp.save.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.save()
        ));

        imp.escape.add_css_class("postio-compose-escape");
        imp.escape.set_hexpand(true);
        imp.escape.set_xalign(1.0);
        // #692: the least essential of the four, so it gives way under a narrow allocation instead
        // of the row overflowing the window.
        imp.escape.set_ellipsize(gtk::pango::EllipsizeMode::End);

        row.append(&imp.send);
        row.append(&imp.schedule_send);
        row.append(&imp.save);
        row.append(&imp.escape);
        self.set_keymap(Keymap::defaults());
        row
    }

    /// Hints follow the user's `[keys]` rebinds (#828). A cleared binding loses its hint rather
    /// than showing a blank.
    pub fn set_keymap(&self, keymap: &Keymap) {
        let imp = self.imp();
        // One at a time: `schedule_send` is a `MenuButton` and the others are `Button`s, so there
        // is no one element type to iterate.
        let hints = action_hints(keymap);
        let child = |index: usize| -> gtk::Widget {
            let (_, text) = ACTION_BUTTONS[index];
            keyhint::labelled(text, hints[index].1.as_deref())
        };
        imp.send.set_child(Some(&child(0)));
        imp.schedule_send.set_child(Some(&child(1)));
        imp.save.set_child(Some(&child(2)));

        while let Some(old) = imp.attach_hint.first_child() {
            imp.attach_hint.remove(&old);
        }
        imp.attach_hint.append(&keyhint::labelled(
            "Attach another",
            hints::key(keymap, CommandId::AttachFile).as_deref(),
        ));

        // `Back` is what `Esc` runs here: it closes the composer and keeps the draft.
        let escape = hints::hint(keymap, CommandId::Back, "keeps the draft");
        imp.escape.set_label(&hints::line(escape.iter()));
        imp.escape.set_visible(escape.is_some());
    }

    /// Test-only: integration tests are a separate compilation unit and cannot reach private
    /// fields.
    #[doc(hidden)]
    pub fn test_set_subject(&self, text: &str) {
        self.imp().subject.set_text(text);
    }

    #[doc(hidden)]
    pub fn test_subject(&self) -> String {
        self.imp().subject.text().to_string()
    }

    #[doc(hidden)]
    pub fn test_focus_field(&self, field: Field) -> bool {
        self.widget_for(field).grab_focus()
    }

    fn widget_for(&self, field: Field) -> gtk::Widget {
        let imp = self.imp();
        match field {
            Field::To => imp.to.clone().upcast(),
            Field::Cc => imp.cc.clone().upcast(),
            Field::Bcc => imp.bcc.clone().upcast(),
            Field::Subject => imp.subject.clone().upcast(),
            Field::Body => imp.body.widget().clone().upcast(),
        }
    }

    #[doc(hidden)]
    pub fn test_schedule_send_button(&self) -> gtk::MenuButton {
        self.imp().schedule_send.clone()
    }

    #[doc(hidden)]
    pub fn test_set_body(&self, text: &str) {
        self.imp().body.test_type(text);
    }

    #[doc(hidden)]
    pub fn test_paste_image_bytes(&self, bytes: Vec<u8>) {
        self.add_inline_image(bytes, "image/png");
    }

    pub fn marks_lit(&self) -> Vec<&'static str> {
        self.imp()
            .format_toggles
            .iter()
            .zip(FORMAT_TOGGLES)
            .filter(|(button, _)| button.is_active())
            .map(|(_, (id, _, _))| id.as_str())
            .collect()
    }

    #[doc(hidden)]
    pub fn test_body_eval(&self, script: &str) -> String {
        self.imp().body.test_eval(script)
    }

    #[doc(hidden)]
    pub fn test_select_body(&self, nth: u32, from: u32, to: u32) {
        self.imp().body.test_select(nth, from, to);
    }

    #[doc(hidden)]
    pub fn test_reply_draft(&self, source: &Message) -> Option<Draft> {
        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        reply_draft(CommandId::Reply, source, &account)
    }

    #[doc(hidden)]
    pub fn test_set_to(&self, text: &str) {
        self.imp().to.set_text(text);
    }

    #[doc(hidden)]
    pub fn test_recipient_popover_visible(&self) -> bool {
        self.imp()
            .to_completion
            .borrow()
            .as_ref()
            .is_some_and(|completion| completion.popover.is_visible())
    }

    #[doc(hidden)]
    pub fn test_accept_recipient_suggestion(&self) -> bool {
        let imp = self.imp();
        let Some(completion) = imp.to_completion.borrow().clone() else {
            return false;
        };
        completion.accept(&imp.to)
    }

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

    /// Calls the handler the entry's key controller calls; whether a real keystroke reaches it is
    /// decided by the propagation phase.
    #[doc(hidden)]
    pub fn test_press_recipient_key(&self, key: gdk::Key) -> bool {
        let imp = self.imp();
        let Some(completion) = imp.to_completion.borrow().clone() else {
            return false;
        };
        completion.handle_key(&imp.to, key) == glib::Propagation::Stop
    }

    #[doc(hidden)]
    pub fn test_recipient_suggestion_count(&self) -> usize {
        self.imp()
            .to_completion
            .borrow()
            .as_ref()
            .map_or(0, |completion| completion.candidates.borrow().len())
    }

    #[doc(hidden)]
    pub fn test_attach_path(&self, path: &std::path::Path) {
        self.add_file(&gio::File::for_path(path));
    }

    #[doc(hidden)]
    pub fn test_attachment_count(&self) -> usize {
        self.imp().draft.borrow().attachments.len()
    }

    #[doc(hidden)]
    pub fn test_attachments_visible(&self) -> bool {
        self.imp().attachments_box.is_visible()
    }

    #[doc(hidden)]
    pub fn test_copy_fields_visible(&self) -> bool {
        let imp = self.imp();
        imp.cc_row.is_visible() && imp.bcc_row.is_visible()
    }

    #[doc(hidden)]
    pub fn test_more_button_visible(&self) -> bool {
        self.imp().more.is_visible()
    }

    /// A drag cannot be synthesised and `drop` takes no boxed `GValue`, so this checks the
    /// controllers are installed on the
    /// right widgets accepting the right type.
    #[doc(hidden)]
    pub fn test_drop_targets(&self) -> Vec<(&'static str, String)> {
        let mut found = Vec::new();
        for (label, widget) in [
            ("composer", self.clone().upcast::<gtk::Widget>()),
            ("body", self.imp().body.widget().clone().upcast()),
        ] {
            let controllers = widget.observe_controllers();
            for index in 0..controllers.n_items() {
                if let Some(target) = controllers.item(index).and_downcast::<gtk::DropTarget>()
                    && let Some(formats) = target.formats()
                {
                    found.push((label, formats.to_string()));
                }
            }
        }
        found
    }

    #[doc(hidden)]
    pub fn test_paste(&self) -> bool {
        self.paste_image()
    }

    #[doc(hidden)]
    pub fn test_insert_image_file(&self, path: &std::path::Path) {
        self.insert_image_file(&gio::File::for_path(path));
    }

    #[doc(hidden)]
    pub fn test_set_cc(&self, text: &str) {
        self.imp().cc.set_text(text);
    }

    #[doc(hidden)]
    pub fn test_remove_attachment(&self, index: usize) {
        self.remove_attachment(index);
    }
}

const FORMAT_TOGGLES: [(CommandId, &str, &str); 6] = [
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
        CommandId::Heading1,
        // Adwaita has no heading icon; Postio draws its own.
        "postio-heading-symbolic",
        "postio-toolbar-heading",
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

/// Title and binding are read from the registry, so the tooltip cannot drift from the palette and
/// cheat sheet.
fn style_toolbar_button(button: &gtk::Button, id: CommandId, icon: &str, class: &str) {
    let spec = postio_core::registry::get(id);
    button.set_icon_name(icon);
    crate::widgets::button::dress_icon(button, spec.title);
    button.add_css_class(class);
    button.set_tooltip_text(Some(&format!(
        "{} ({})",
        spec.title,
        // Expanded first: the registry stores `mod+shift+8`, which a tooltip would read as
        // "Mod+Shift+8".
        pretty_binding(&postio_config::keys::expand_mod(
            spec.default_binding,
            postio_config::paths::Platform::host(),
        ))
    )));
    button.update_property(&[gtk::accessible::Property::Label(spec.title)]);
    // The click acts on the editor's selection; taking focus would collapse it.
    button.set_focus_on_click(false);
}

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

/// The type comes from the shared-mime-info sniff, not the extension; the size cap keeps a camera
/// RAW from ballooning the
/// message (it is attached instead of inlined).
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

fn identity_label(identity: &Identity) -> String {
    if identity.display_name.trim().is_empty() || identity.display_name == identity.address.address
    {
        identity.address.address.clone()
    } else {
        format!("{} <{}>", identity.display_name, identity.address.address)
    }
}

fn field_label(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class("postio-compose-label");
    label.set_xalign(0.0);
    label.set_width_chars(8);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    label
}

const ACTION_BUTTONS: &[(CommandId, &str)] = &[
    (CommandId::Send, "Send"),
    (CommandId::ScheduleSend, "Schedule…"),
    (CommandId::SaveDraft, "Save draft"),
];

/// `None` when the user cleared the binding. A free function so a rebind is testable without a
/// display (#828).
fn action_hints(keymap: &Keymap) -> Vec<(CommandId, Option<String>)> {
    ACTION_BUTTONS
        .iter()
        .map(|(id, _)| (*id, keymap.binding(*id).map(str::to_owned)))
        .collect()
}

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
    crate::widgets::button::dress_icon(button, tooltip);
}

/// One per entry because a `Popover` is parented to the one widget it points at.
pub(crate) struct Completion {
    popover: gtk::Popover,
    list: gtk::ListBox,
    candidates: RefCell<Vec<RecipientCandidate>>,
    /// A hide waiting for the main loop, so a change the next one undoes (`set_text`'s
    /// empty-then-filled, a paste) leaves the
    /// popover up. On Wayland a re-show is a new popup, and one repositioned before it has drawn
    /// waits on a compositor reply:
    /// GTK blocks the main loop.
    hiding: Rc<RefCell<Option<glib::SourceId>>>,
    /// A popup shown a frame after it hid reuses its surface while the renderer may still have a
    /// frame in flight, and the
    /// compositor drops the client ("already has a buffer committed") or GTK waits for ever.
    hidden_at: Rc<Cell<Option<std::time::Instant>>>,
    showing: Rc<RefCell<Option<glib::SourceId>>>,
}

const REAPPEAR_AFTER: std::time::Duration = std::time::Duration::from_millis(150);

impl Completion {
    /// The returned value need not be kept: `entry`'s signal connections hold it until the field is
    /// destroyed.
    fn install(composer: &Composer, entry: &gtk::Entry) -> Rc<Self> {
        let list = gtk::ListBox::new();
        list.set_selection_mode(gtk::SelectionMode::Browse);
        list.add_css_class("postio-recipient-suggestions");
        list.set_accessible_role(gtk::AccessibleRole::ListBox);

        let popover = gtk::Popover::new();
        // Autohide would take the grab from the entry still being typed into.
        popover.set_autohide(false);
        popover.set_has_arrow(false);
        popover.set_position(gtk::PositionType::Bottom);
        popover.add_css_class("postio-recipient-completion");
        popover.set_child(Some(&list));
        popover.set_parent(entry);
        // A custom child: GTK requires unparenting it before the entry is finalized, or it warns
        // and leaks the popover.
        entry.connect_destroy({
            let popover = popover.downgrade();
            move |_| {
                if let Some(popover) = popover.upgrade() {
                    popover.unparent();
                }
            }
        });

        let this = Rc::new(Self {
            popover,
            list,
            candidates: RefCell::new(Vec::new()),
            hiding: Rc::default(),
            hidden_at: Rc::default(),
            showing: Rc::default(),
        });
        this.popover.connect_closed({
            let hidden_at = Rc::clone(&this.hidden_at);
            move |_| hidden_at.set(Some(std::time::Instant::now()))
        });

        entry.connect_changed(glib::clone!(
            #[weak]
            composer,
            #[strong]
            this,
            move |entry| this.update(&composer, entry)
        ));

        this.list.connect_row_activated({
            // A strong clone would keep the popover and list alive after the composer is destroyed.
            let this = Rc::downgrade(&this);
            let entry = entry.downgrade();
            move |_, row| {
                if let (Some(this), Some(entry)) = (this.upgrade(), entry.upgrade()) {
                    this.accept_row(&entry, row);
                }
            }
        });

        let keys = gtk::EventControllerKey::new();
        // Capture: the focus widget is the `GtkText` inside the entry, whose Return binding
        // consumes the event before a
        // bubble-phase controller runs (#424).
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

    fn update(&self, composer: &Composer, entry: &gtk::Entry) {
        let text = entry.text();
        let (_, token) = current_entry(&text);
        // Below the threshold nothing is looked up: the provider is a database query. Counted in
        // chars, not bytes.
        if token.chars().count() < MIN_COMPLETION_PREFIX {
            self.hide_soon();
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
            self.hide_soon();
        } else {
            if let Some(hiding) = self.hiding.take() {
                hiding.remove();
            }
            self.list.select_row(self.list.row_at_index(0).as_ref());
            self.show_settled();
        }
    }

    fn show_settled(&self) {
        if self.popover.is_visible() || self.showing.borrow().is_some() {
            return;
        }
        let since = self.hidden_at.get().map(|at| at.elapsed());
        let wait = since.map_or(std::time::Duration::ZERO, |since| {
            REAPPEAR_AFTER.saturating_sub(since)
        });
        if wait.is_zero() {
            self.popover.popup();
            return;
        }
        let popover = self.popover.downgrade();
        let list = self.list.downgrade();
        let showing = Rc::clone(&self.showing);
        let source = glib::timeout_add_local_once(wait, move || {
            showing.take();
            if let (Some(popover), Some(list)) = (popover.upgrade(), list.upgrade())
                && list.row_at_index(0).is_some()
                && !popover.is_visible()
            {
                popover.popup();
            }
        });
        self.showing.replace(Some(source));
    }

    fn hide_soon(&self) {
        if let Some(showing) = self.showing.take() {
            showing.remove();
        }
        if !self.popover.is_visible() || self.hiding.borrow().is_some() {
            return;
        }
        let popover = self.popover.downgrade();
        let hiding = Rc::clone(&self.hiding);
        let source = glib::idle_add_local_once(move || {
            hiding.take();
            if let Some(popover) = popover.upgrade() {
                popover.popdown();
            }
        });
        self.hiding.replace(Some(source));
    }

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

    fn accept(&self, entry: &gtk::Entry) -> bool {
        let Some(row) = self.list.selected_row() else {
            return false;
        };
        self.accept_row(entry, &row)
    }

    /// Takes the row rather than re-reading the selection: a click must commit what was under the
    /// pointer.
    fn accept_row(&self, entry: &gtk::Entry, row: &gtk::ListBoxRow) -> bool {
        let Some(candidate) = self.candidates.borrow().get(row.index() as usize).cloned() else {
            return false;
        };

        let replaced = postio_ui::recipients::accepted(&entry.text(), &candidate);
        entry.set_text(&replaced);
        entry.set_position(-1);
        self.popover.popdown();
        true
    }
}

/// HTML is the record (ADR 0004), so a draft with one is parsed from it; the text half is for plain
/// messages.
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
    fn a_draft_with_recipient_and_subject_starts_in_the_body() {
        let mut draft = Draft::new(AccountId::new(1));
        assert_eq!(starting_field(&draft), Field::To, "empty To comes first");
        draft.subject = "Notes".to_owned();
        assert_eq!(starting_field(&draft), Field::To, "nobody to send it to");
        draft.to = vec![EmailAddress::new(Some("Ada"), "ada@example.com")];
        assert_eq!(starting_field(&draft), Field::Body);
        draft.subject.clear();
        assert_eq!(starting_field(&draft), Field::To, "no subject yet");
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

    fn written() -> Draft {
        let mut draft = draft();
        draft.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        draft.subject = "the mbox importer".to_owned();
        draft.body = MessageBody {
            text: Some("Looking now.".to_owned()),
            html: None,
        };
        draft
    }

    #[test]
    fn compose_starts_a_blank_message_even_when_a_draft_was_kept() {
        assert_eq!(opening(&written(), &draft()), Opening::Fill);
    }

    #[test]
    fn every_request_opens_what_it_asked_for() {
        let mut reply = draft();
        reply.kind = DraftKind::Reply;
        let mut forward = draft();
        forward.kind = DraftKind::Forward;

        for asked in [draft(), written(), reply, forward] {
            assert_eq!(opening(&written(), &asked), Opening::Fill);
            assert_eq!(opening(&draft(), &asked), Opening::Fill);
        }
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
    fn a_message_with_nothing_odd_about_it_is_not_questioned() {
        let mut ordinary = draft();
        ordinary.subject = "the tide gate".to_owned();
        ordinary.body.text = Some("Armed and holding.".to_owned());
        assert!(send_concerns(&ordinary).is_empty());
    }

    #[test]
    fn an_empty_subject_is_asked_about_and_never_refused() {
        let mut bare = draft();
        bare.subject = String::new();
        bare.body.text = Some("Armed and holding.".to_owned());

        let concerns = send_concerns(&bare);
        assert_eq!(concerns.len(), 1, "{concerns:?}");
        assert!(concerns[0].contains("subject"), "{concerns:?}");

        bare.subject = "   ".to_owned();
        assert_eq!(send_concerns(&bare).len(), 1);
    }

    #[test]
    fn two_odd_things_about_one_message_are_one_question() {
        let mut both = draft();
        both.subject = String::new();
        both.body.text = Some("Please find attached.".to_owned());

        let concerns = send_concerns(&both);
        assert_eq!(concerns.len(), 2, "{concerns:?}");

        let said = join_with_and(&concerns);
        assert!(said.contains(" and "), "the clauses are not joined: {said}");
        assert!(
            said.contains("subject") && said.contains("attachment"),
            "{said}"
        );
        assert!(
            !said.contains(" and  and "),
            "the joiner doubled up: {said}"
        );
    }

    #[test]
    fn implausible_recipients_are_named_not_just_counted() {
        let mut one = draft();
        one.to = vec![
            EmailAddress::new(None::<String>, "ada@example.com"),
            EmailAddress::new(None::<String>, "grace"),
        ];
        let said = recipient_warning(&one).expect("one address is wrong");
        assert!(said.contains("grace"), "the address is not named: {said}");
        assert!(said.contains("To"), "the field is not named: {said}");
        assert!(
            !said.contains("ada@example.com"),
            "an address that is fine must not be named as a problem: {said}"
        );

        one.cc = vec![EmailAddress::new(None::<String>, "@example.com")];
        let said = recipient_warning(&one).expect("two addresses are wrong");
        assert!(
            said.contains("grace") && said.contains("@example.com"),
            "{said}"
        );
        assert!(said.contains("To") && said.contains("Cc"), "{said}");

        let mut fine = draft();
        fine.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        assert_eq!(recipient_warning(&fine), None);
    }

    #[test]
    fn a_long_list_of_wrong_addresses_is_summarised_rather_than_recited() {
        let mut many = draft();
        many.to = (0..9)
            .map(|n| EmailAddress::new(None::<String>, format!("wrong{n}")))
            .collect();
        let said = recipient_warning(&many).expect("nine addresses are wrong");
        assert!(said.contains('9'), "the total is not stated: {said}");
        assert!(
            said.len() < 200,
            "the warning recites every address instead of summarising: {said}"
        );
    }

    #[test]
    fn the_recipient_count_is_reported_per_field_before_sending() {
        let mut wide = draft();
        wide.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        wide.cc = (0..41)
            .map(|n| EmailAddress::new(None::<String>, format!("person{n}@example.org")))
            .collect();
        wide.bcc = vec![EmailAddress::new(None::<String>, "quiet@example.net")];

        let said = recipient_summary(&wide).expect("a wide message says so");
        assert!(said.contains("41"), "the Cc count is missing: {said}");
        assert!(said.contains("Cc"), "the field is not named: {said}");
        assert!(said.contains("Bcc"), "Bcc is not counted: {said}");

        let mut narrow = draft();
        narrow.to = vec![EmailAddress::new(None::<String>, "ada@example.com")];
        assert_eq!(recipient_summary(&narrow), None);
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
        // Not a suffix match on the bare string: "notshop.example.org" is not a subdomain of
        // "shop.example.org".
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
        let mut source = html_only_source_with_a_tracking_link();
        source.body = MessageBody {
            text: Some("The lamp has shipped".to_owned()),
            html: Some("<p>The lamp <strong>has shipped</strong></p>".to_owned()),
        };
        let account = Account::new("Test", EmailAddress::new(None::<String>, "you@example.net"));
        let draft = reply_draft(CommandId::Reply, &source, &account).expect("a reply");

        let document = document_of(&draft.body);
        let quoted = document
            .blocks
            .iter()
            .find_map(|block| match block {
                postio_body::Block::Quoted(quoted) => Some(quoted),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no quote in the reply: {document:?}"));
        assert!(
            quoted.html().contains("<strong>"),
            "the emphasis was flattened out of the quote: {}",
            quoted.html()
        );
        let text = draft.body.text.expect("a text half");
        assert!(text.contains("> The lamp"), "{text}");
        assert!(text.contains("wrote:"), "{text}");
    }

    #[test]
    fn replying_to_your_own_flowed_plain_text_message_unwraps_the_soft_breaks() {
        // #456: a format=flowed plain-text message whose sentence wraps across three lines must
        // quote as one sentence.
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
        let quoted = document
            .blocks
            .iter()
            .find_map(|block| match block {
                postio_body::Block::Quoted(quoted) => Some(quoted),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no quote in the reply: {document:?}"));
        assert!(
            quoted.text().contains(sentence),
            "the quote must be the sender's one sentence, not their soft \
             wrap read back as typed line breaks: {:?}",
            quoted.text()
        );
    }

    #[test]
    fn replying_to_an_ordinary_senders_short_lines_leaves_them_as_typed() {
        // A message with short lines and no format=flowed parameter must not be reflowed.
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
        let quoted = document
            .blocks
            .iter()
            .find_map(|block| match block {
                postio_body::Block::Quoted(quoted) => Some(quoted),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no quote in the reply: {document:?}"));
        assert!(
            quoted.html().contains("walkthrough.<br>See you then."),
            "an ordinary sender's own line break must survive as a break, \
             not be joined onto its neighbour: {}",
            quoted.html()
        );
        assert!(
            quoted.text().contains("walkthrough.\nSee you then."),
            "and the plain half must keep it too: {:?}",
            quoted.text()
        );
    }

    #[test]
    fn a_forward_of_a_hostile_message_carries_no_load_and_no_script() {
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
        let now = local_at(2024, 1, 1, 9, 0);
        let presets = schedule_presets(now);

        assert_eq!(preset(&presets, "In 1 hour"), local_at(2024, 1, 1, 10, 0));
        assert_eq!(
            preset(&presets, "Later today"),
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
    fn the_evening_rolls_to_tomorrow_and_says_so_once_it_has_passed() {
        let now = local_at(2024, 1, 1, 19, 0);
        let presets = schedule_presets(now);

        assert_eq!(
            preset(&presets, "Tomorrow evening"),
            local_at(2024, 1, 2, 18, 0),
            "6pm today is behind a 7pm picker"
        );
    }

    #[test]
    fn monday_morning_is_the_nearest_monday_still_ahead() {
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

    fn keys_of(keymap: &Keymap) -> Vec<Option<String>> {
        action_hints(keymap)
            .into_iter()
            .map(|(_, key)| key)
            .collect()
    }

    #[test]
    fn the_action_row_takes_its_keys_from_the_registry() {
        assert_eq!(
            keys_of(Keymap::defaults()),
            vec![
                Some("ctrl+Return".to_string()),
                Some("ctrl+shift+Return".to_string()),
                Some("ctrl+s".to_string()),
            ],
            "canvas order: Send, Schedule, Save draft"
        );
    }

    #[test]
    fn a_rebind_in_keys_reaches_the_action_row() {
        // A literal names the default key for ever, so the button lies to whoever rebound it.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("send".to_string(), "mod+Return".to_string());
        overrides
            .overrides_mut()
            .insert("save_draft".to_string(), "mod+w".to_string());

        let keys = keys_of(&Keymap::resolve(&overrides));
        assert_eq!(keys[2], Some("ctrl+w".to_string()), "Save draft rebound");
        assert_eq!(
            keys[1],
            Some("ctrl+shift+Return".to_string()),
            "Schedule kept its default; only the two named were rebound"
        );
    }

    #[test]
    fn a_command_with_no_key_left_shows_no_hint_rather_than_a_blank_one() {
        // `save_draft` has one key and no alternate, so it is the verb that can still be emptied;
        // an explicit `[keys]` entry
        // outranks a default.
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("send".to_string(), "mod+s".to_string());

        let keys = keys_of(&Keymap::resolve(&overrides));
        assert_eq!(
            keys[0],
            Some("ctrl+s".to_string()),
            "the override wins the key"
        );
        assert_eq!(
            keys[2], None,
            "and Save draft shows no hint at all: {keys:?}"
        );
    }
}
