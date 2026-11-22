//! The composer: canvas 2a, taking over the reading pane rather than opening a window
//! (docs/PRODUCT.md §10), so the list keeps its scroll and selection.
//! `Esc` closes the composer and the draft stays in it; discarding is a separate destructive verb
//! that asks first (`closing` is the rule).
//! Until a handler is connected the composer leaves the draft in place and says so on the status
//! line, rather than lose the mail into an unconnected seam.

pub mod editor;
pub mod scheme;
pub mod source_view;
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

/// A reply focuses the body (recipients and subject are decided); new mail focuses `To`.
/// The other fields exist so the focus order can be asserted (FR-003).
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

/// The app's vocabulary: Compose, never "New" or "Write".
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

/// The draft asked for always wins (#1196, #691). A kept draft is autosaved into Drafts, so
/// starting fresh loses nothing;
/// `Composer::resume` opens it by name. Still called so a reader of `opening(..) == Restore` can
/// find why it never is.
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
        // Named, not counted (FR-024): a count alone leaves the user to find the wrong one among
        // nine.
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

/// `None` for the ordinary case: a banner that is always there is one nobody reads.
pub fn recipient_summary(draft: &Draft) -> Option<String> {
    postio_ui::compose::recipient_summary(draft.to.len(), draft.cc.len(), draft.bcc.len())
}

fn fields(draft: &Draft) -> [(&'static str, &[EmailAddress]); 3] {
    [
        ("To", draft.to.as_slice()),
        ("Cc", draft.cc.as_slice()),
        ("Bcc", draft.bcc.as_slice()),
    ]
}

/// A subdomain counts as the same organisation: `click.shop.example.org` linking from
/// `shop.example.org` is not what `quoted_tracking_domains` flags.
fn differs_from_sender(host: &str, sender_domain: &str) -> bool {
    let host = host.to_ascii_lowercase();
    let sender_domain = sender_domain.to_ascii_lowercase();
    host != sender_domain && !host.ends_with(&format!(".{sender_domain}"))
}

/// Purely a notice: no link is stripped or rewritten upstream (a link is not a load) (#116).
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

/// A reply reaches people the original did not, and a tracking redirect usually encodes the first
/// recipient's id.
/// Informational only (#116). Scans what the quote will contain, via the same `source_document` the
/// reply is built from (#340).
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

/// One constant because the scroll region around it is a separate tab stop and must say the same
/// thing.
const BODY_NAME: &str = "Message body";

const UNSAVED: &str = postio_ui::compose::NOT_SAVED;

const RETAINED: &str = "still holding the draft Esc kept";

/// Reachable only with no account configured: the composition root connects `connect_send` whenever
/// it has one (#423).
const NO_SEND_PATH: &str = "not sent — no outgoing account is connected yet";

/// `open` refuses to replace a draft in progress, as it does a retained one; the refusal must say
/// so, since silence looked like the key doing nothing (#426).
const REPLY_BLOCKED: &str = "not opened — finish or close the current draft first";

const NO_ATTACH_PATH: &str = "not attached — no attachment handler is connected yet";

/// What the picker's first entry says: sign the way this identity signs.
const SIGNATURE_FROM_IDENTITY: &str = "Identity signature";

const NO_INLINE_PATH: &str = "not inserted — no inline-image handler is connected yet";

/// A fixed default until the configurable threshold is wired through `postio-config`.
const LARGE_ATTACHMENT_BYTES: u64 = 25 * 1024 * 1024;

/// Long enough that a burst of keystrokes coalesces into one save, short enough that a crash loses
/// little.
const AUTOSAVE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(1500);

/// Everything the composer asks its app for goes through here, so the composer names no app and a
/// second host is a second implementation.
pub trait ComposerHost {
    fn parent(&self) -> Option<gtk::Window>;
    fn install(&self, composer: &Composer);
    fn restore(&self, composer: &Composer);
    fn remove(&self, composer: &Composer);
    /// `None` for a host with no frame. Detaching reparents the same widgets, nothing rebuilt.
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

/// Narrower than the main window: one message being written, not a mail client.
const DETACHED_SIZE: (i32, i32) = (620, 560);

type DraftHandler = Box<dyn Fn(&Draft)>;

/// Carries the chosen time: nothing downstream can ask the picker again once the draft is handed
/// off.
type SendLaterHandler = Box<dyn Fn(&Draft, DateTime<Utc>)>;

/// `&mut` because the storage handler assigns a `DraftId` on first save; `Composer::save` writes it
/// back so later saves update the same row.
type SaveHandler = Box<dyn Fn(&mut Draft)>;

type ClosedHandler = Box<dyn Fn(Closing)>;

type OpenedHandler = Box<dyn Fn()>;

pub use postio_model::contact_group::RecipientCandidate;

type RecipientSuggestions = Box<dyn Fn(&str) -> Vec<RecipientCandidate>>;

/// `None` means nothing to reply to, and the keystroke does nothing. Answered through a callback
/// (#1608): the lookup is a store read and must not block drawing.
type ReplySourceProvider = Box<dyn Fn(ReplyAnswer)>;

pub type ReplyAnswer = Box<dyn FnOnce(Option<(Message, Account)>)>;

/// `None` means neither the mailbox nor the account has an opinion. Answered through a callback, as
/// `ReplySourceProvider` is (#1608).
type SignatureDefaultProvider = Box<dyn Fn(SignatureAnswer)>;

pub type SignatureAnswer = Box<dyn FnOnce(Option<SignatureId>)>;

/// Calling this adds the row, synchronously or from a spawned task's callback. `pub` because it
/// appears in `connect_attach`'s signature.
pub type AttachReady = Box<dyn FnOnce(Option<Attachment>)>;

pub type InlineImageReady = Box<dyn FnOnce(Option<Attachment>)>;

/// Calls `InlineImageReady` whenever ready: the same non-blocking contract as `AttachHandler`.
type InlineImageHandler = Box<dyn Fn(Vec<u8>, String, InlineImageReady)>;

pub type RemoteImageReady = Box<dyn FnOnce(Result<Attachment, postio_ui::compose::RemoteRefusal>)>;

type RemoteImageHandler =
    Box<dyn Fn(postio_model::DraftId, postio_body::WebHref, RemoteImageReady)>;

#[derive(Clone, Debug, PartialEq, Eq)]
enum RemoteState {
    Downloading,
    Refused(String),
}

type BlobLookup = Rc<RefCell<Option<Box<dyn Fn(&str) -> Option<(Vec<u8>, String)>>>>>;

/// Synchronous and local, like `crate::reader::BlobSource`, because that is what a scheme handler
/// can await.
type AttachmentBytes = Box<dyn Fn(&Attachment) -> Option<Vec<u8>>>;

/// A handler that blocks reading a large file blocks the composer: slow work belongs on a task the
/// handler spawns, calling `AttachReady` when done.
type AttachHandler = Box<dyn Fn(std::path::PathBuf, AttachReady)>;

/// Only when the composer is not already open: replying to a reply in progress is not a thing.
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
        /// `None` is "no limit configured", not "unknown": nothing is checked.
        pub size_limit: Cell<Option<u64>>,
        pub cc: gtk::Entry,
        pub bcc: gtk::Entry,
        pub subject: gtk::Entry,
        pub cc_row: gtk::Box,
        pub bcc_row: gtk::Box,
        pub more: gtk::Button,
        /// Its label flips: one control that says which way it goes beats two that are each wrong
        /// half the time.
        pub detach: gtk::Button,
        /// `Some` is the whole of "detached": a separate flag could drift from the window.
        pub detached: RefCell<Option<adw::Window>>,
        pub identity_row: gtk::Box,
        /// Entry 0 is the identity's own; the rest are the account's set (#12).
        pub signatures: RefCell<Vec<Signature>>,
        pub signature: gtk::DropDown,
        pub identity: gtk::DropDown,
        pub identity_only: gtk::Label,
        /// The `Editor` owns the document, edit history and typing-run coalescing; the composer
        /// holds a view over its record (ADR 0004 Q6, #347).
        pub body: editor::Editor,
        pub source: source_view::SourceView,
        pub source_toggle: gtk::ToggleButton,
        pub source_notice: gtk::Box,
        pub source_notice_label: gtk::Label,
        pub source_notice_undo: gtk::Button,
        pub source_typed: RefCell<Option<String>>,
        pub format_toggles: [gtk::ToggleButton; 6],
        pub link_button: gtk::Button,
        pub image_button: gtk::Button,
        pub attach_button: gtk::Button,
        pub send: gtk::Button,
        /// The popover is rebuilt each time it opens: the presets are relative to the moment it
        /// opens.
        pub schedule_send: gtk::MenuButton,
        pub save: gtk::Button,
        /// The least essential of the four (#692): it ellipsizes under a narrow allocation.
        pub escape: gtk::Label,
        pub attach_hint: gtk::Box,
        pub warning: gtk::Label,
        /// A separate label from `warning`: the two can be true at once and are about unrelated
        /// things (#116).
        pub tracking_notice: gtk::Label,
        pub attachments_box: gtk::Box,
        pub below_body: gtk::Box,
        pub attachments_list: gtk::ListBox,
        /// Not part of the draft: it is what the fetch is doing, and a new composition starts with
        /// none (#1690).
        pub downloading: RefCell<std::collections::BTreeSet<String>>,
        pub draft: RefCell<Draft>,
        /// Kept apart from the draft cell: reading it off the cell made a reply's account become
        /// where the next new message came from (#960, #1161).
        pub composing_from: Cell<AccountId>,
        pub identities: RefCell<Vec<Identity>>,
        pub sent: RefCell<Vec<DraftHandler>>,
        pub sent_later: RefCell<Vec<SendLaterHandler>>,
        pub saved: RefCell<Vec<SaveHandler>>,
        pub changed: RefCell<Vec<DraftHandler>>,
        pub closed: RefCell<Vec<ClosedHandler>>,
        pub opened: RefCell<Vec<OpenedHandler>>,
        /// One slot, last registration wins: there is exactly one reading pane.
        pub reply_source: RefCell<Option<ReplySourceProvider>>,
        /// Read when `c` starts a draft, since the selected mailbox changes more often than the
        /// account (#394).
        pub signature_default: RefCell<Option<SignatureDefaultProvider>>,
        pub recipient_suggestions: RefCell<Option<RecipientSuggestions>>,
        pub signature_placement: Cell<(Placement, Placement)>,
        pub attach: RefCell<Option<AttachHandler>>,
        pub inline_image: RefCell<Option<InlineImageHandler>>,
        pub(super) remote_image: RefCell<Option<super::RemoteImageHandler>>,
        pub(super) remote_state: RefCell<std::collections::HashMap<String, super::RemoteState>>,
        pub remote_box: gtk::Box,
        pub remote_list: gtk::ListBox,
        pub(super) remote_drawn: RefCell<Vec<(String, String, Option<super::RemoteState>)>>,
        pub remote_warning: gtk::Label,
        pub attachment_bytes: RefCell<Option<AttachmentBytes>>,
        /// Held here because the editor is constructed before the composer object exists.
        pub blob_lookup: BlobLookup,
        pub host: RefCell<Option<Rc<dyn super::ComposerHost>>>,
        /// Bumped every time the fields are filled with a draft; an answer arriving for an older
        /// one is ignored (#1608). See `Composer::adopt_id`.
        pub generation: Cell<u64>,
        /// The composition before the last fill: a close or send just finished it, since both
        /// refill before the closed handlers run.
        pub previous_generation: Cell<u64>,
        pub filling: Cell<bool>,
        pub held: RefCell<Vec<postio_body::Block>>,
        pub began_as: RefCell<Option<Draft>>,
        pub autosave_source: Cell<Option<glib::SourceId>>,
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
                source: source_view::SourceView::new(),
                source_toggle: gtk::ToggleButton::new(),
                source_notice: gtk::Box::new(
                    gtk::Orientation::Horizontal,
                    crate::widgets::space::S2,
                ),
                source_notice_label: gtk::Label::new(None),
                source_notice_undo: gtk::Button::with_label("Undo"),
                source_typed: RefCell::new(None),
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
                remote_image: RefCell::new(None),
                remote_state: RefCell::default(),
                remote_box: gtk::Box::new(gtk::Orientation::Vertical, crate::widgets::space::S2),
                remote_list: gtk::ListBox::new(),
                remote_drawn: RefCell::default(),
                remote_warning: gtk::Label::new(None),
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

    /// Safe to call more than once and a no-op once anything has been loaded, so it cannot discard
    /// a draft.
    /// The first load starts a WebKit web process, so call it on an idle turn once the window is up
    /// (#1216).
    pub fn warm(&self) {
        self.imp().body.warm();
    }

    pub fn is_warm(&self) -> bool {
        self.imp().body.is_warm()
    }

    pub fn open(&self, draft: Draft) {
        // Already composing: `c` again is a no-op that puts the keyboard back (in the detached
        // window if detached), never a reset.
        if self.is_open() {
            if let Some(host) = self.detached_window() {
                host.present();
            }
            self.focus_first();
            return;
        }

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

    /// Unlike `open`, this replaces a retained draft: it is autosaved into Drafts since #166, so
    /// swapping loses nothing.
    /// Pending autosave is flushed first: the timer would otherwise write one draft's words onto
    /// another's row.
    pub fn resume(&self, draft: Draft) {
        // `is_assigned`, not just equality: every unsaved draft carries `DraftId::UNASSIGNED`
        // (#1240).
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
        } else if holds(&focus, imp.body.widget().upcast_ref())
            || holds(&focus, imp.source.text_view().upcast_ref())
        {
            Some(Field::Body)
        } else {
            None
        }
    }

    /// Never destroys typed content; that is `Composer::discard`, and it asks first.
    pub fn close(&self) -> Closing {
        // The pending autosave belongs to the composition being closed: after the refill it saved
        // the empty draft (#1608).
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
        self.flush_autosave();
        // Reparented before hiding, never after: a widget with no parent has no root, and
        // `focused_field` and the pane restore ask the root.
        self.reclaim();
        self.set_visible(false);
        self.release_pane();
        for handler in self.imp().closed.borrow().iter() {
            handler(outcome);
        }
    }

    pub fn discard(&self) {
        // The configured account, not the discarded draft's: a discarded reply must not leave its
        // account for the next `c`.
        let account = self.account();
        self.fill(Draft::new(account));
        self.close();
    }

    /// The one composer verb the registry marks `Recovery::Confirm`: typed prose exists nowhere
    /// else. An empty draft is discarded without asking.
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
        // The rich view writes HTML, so Markdown left on the draft would describe a message that no
        // longer exists; the Markdown view keeps the person's own spelling (FR-028).
        draft.body_markdown = imp
            .source
            .is_shown()
            .then(|| postio_ui::editor::source::markdown_of(&imp.source.text()));
        draft
    }

    /// `html` only when the document carries structure `to_text` would lose; always regenerated,
    /// never carried over from the opened draft.
    /// `text` is `to_flowed_text` (#333): wrapping happens once, here, before filing and sending.
    /// `source_document` undoes it on read via `Message::text_is_flowed` (#456).
    fn body(&self) -> MessageBody {
        postio_body::message_body(&self.document())
    }

    fn body_edited(&self) {
        if self.imp().filling.get() {
            return;
        }
        // A "for now" answer lasts until the next edit; a narrowing notice, which has an Undo,
        // until the next switch.
        let imp = self.imp();
        if imp.source_notice.is_visible() && !imp.source_notice_undo.is_visible() {
            imp.source_notice.set_visible(false);
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

    /// The Editor's record; the surface is a copy of it, never the reverse (ADR 0004).
    pub fn document(&self) -> postio_body::Document {
        let imp = self.imp();
        let mut document = if imp.source.is_shown() {
            postio_ui::editor::source::close(&imp.source.text(), &imp.source.quotes(), None)
                .document
        } else {
            imp.body.document()
        };
        document
            .blocks
            .extend(self.imp().held.borrow().iter().cloned());
        document
    }

    /// Empty when the body forwards no thread. The folded messages stay part of the draft.
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

    /// One identity is not a choice, so it is a line of text, not a picker. The default is selected
    /// and its signature goes in straight away.
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

    /// The override is the draft's: recorded on it, so reopening sends from the same address, and a
    /// later reply picks its own identity.
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

    /// `None` means no configured limit and nothing is checked: a guessed ceiling refuses mail the
    /// provider would have taken.
    pub fn set_size_limit(&self, limit: Option<u64>) {
        self.imp().size_limit.set(limit);
    }

    /// Checked here rather than left to the server (FR-056): a rejection arrives after the composer
    /// has closed.
    fn too_large(&self) -> Option<postio_model::size::TooLarge> {
        postio_model::size::check(&self.draft(), self.imp().size_limit.get())
    }

    /// Hidden when the account has none: a picker with one entry says only what is already true.
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
        self.imp().signature.set_selected((index + 1) as u32);
        true
    }

    /// "Neither has an opinion" resets the picker to the identity's own, not an older draft's
    /// choice (#394).
    fn open_new_draft(&self) {
        self.open(Draft::new(self.account()));
        let generation = self.generation();
        let weak = self.downgrade();
        let answer: SignatureAnswer = Box::new(move |resolved| {
            let Some(composer) = weak.upgrade() else {
                return;
            };
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
        // An empty picker reports `INVALID_LIST_POSITION`, not zero: entry 0 and "no entries" both
        // mean the identity's own signature.
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

    /// Does not touch the body (FR-031): the `From` dropdown must not rewrite a hand-edited
    /// signature, which `apply_signature` cannot tell from an untouched one.
    fn apply_identity(&self) {
        let Some(identity) = self.identity() else {
            return;
        };
        let imp = self.imp();
        let mut draft = self.draft();
        draft.use_identity(&identity);
        imp.draft.borrow_mut().identity_id = draft.identity_id;
    }

    /// Run when a draft opens and when the signature picker is used; never on an identity change
    /// (see `apply_identity`).
    fn apply_signature(&self) {
        let Some(identity) = self.identity() else {
            return;
        };
        let imp = self.imp();

        // At the block level for every draft, never through text: flattening a rich quote to `> `
        // lines would destroy the reply (#340).
        let current = imp.body.document();
        let signature = self.chosen_signature(&identity);
        let wanted =
            postio_body::apply_signature(&current, signature.as_ref(), self.signature_placement());
        if current == wanted {
            return;
        }

        // The caret goes to the start of the body; preserving the exact offset across an Editor
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

    /// Set by whoever did it: the composer has nowhere to save to and never claims it did.
    pub fn set_status(&self, status: &str) {
        self.imp().status.set_text(status);
    }

    /// With nothing connected the draft stays and the status line names what is missing.
    /// Checks what the Send button's `is_sendable` checks, since `ctrl+Return` reaches this
    /// directly: no recipients would empty the composer and drain as impossible, and an
    /// already-queued draft resumed from Drafts would send twice.
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
        // Asked rather than refused (FR-018, FR-057): a message can legitimately have no subject.
        // One question for however many concerns apply.
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

    /// What is on the other side is irreversible and no undo serves. One dialog for all concerns,
    /// so nobody learns to answer the second unread.
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

    /// The same sendability checks as `send`.
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

    /// A handler may assign the draft an id on its first persisted save; it is written back so the
    /// next save updates the same row. Nothing else a handler touches is kept.
    pub fn save(&self) {
        let mut draft = self.draft();
        // Nothing written and never saved: not worth a row. The editor reports asynchronously, so
        // an edit can re-arm the autosave after a close and insert an empty `Editing` row (#1608).
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

    /// Saves are written off the main thread, so the id arrives after `save` returned. Taken only
    /// while the same composition is in the fields and has no id: another draft's next save must
    /// not update this row (#1608).
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

    /// Not when `c` finds it already open and moves the keyboard back.
    pub fn connect_opened(&self, handler: impl Fn() + 'static) {
        self.imp().opened.borrow_mut().push(Box::new(handler));
    }

    pub fn connect_reply_source(&self, provider: impl Fn(ReplyAnswer) + 'static) {
        *self.imp().reply_source.borrow_mut() = Some(Box::new(provider));
    }

    /// Called at the moment `c` starts a draft so it answers for the mailbox selected right then
    /// (#394).
    pub fn connect_signature_default(&self, provider: impl Fn(SignatureAnswer) + 'static) {
        *self.imp().signature_default.borrow_mut() = Some(Box::new(provider));
    }

    /// The composer only shows what comes back, in that order, and never touches the network:
    /// completion is local.
    pub fn connect_recipient_suggestions(
        &self,
        provider: impl Fn(&str) -> Vec<RecipientCandidate> + 'static,
    ) {
        *self.imp().recipient_suggestions.borrow_mut() = Some(Box::new(provider));
    }

    /// `handler` may call the callback immediately or from a background task; `add_file` never
    /// blocks on it.
    pub fn connect_attach(&self, handler: impl Fn(std::path::PathBuf, AttachReady) + 'static) {
        *self.imp().attach.borrow_mut() = Some(Box::new(handler));
    }

    pub fn connect_inline_image(
        &self,
        handler: impl Fn(Vec<u8>, String, InlineImageReady) + 'static,
    ) {
        *self.imp().inline_image.borrow_mut() = Some(Box::new(handler));
    }

    pub fn connect_remote_image(
        &self,
        handler: impl Fn(postio_model::DraftId, postio_body::WebHref, RemoteImageReady) + 'static,
    ) {
        *self.imp().remote_image.borrow_mut() = Some(Box::new(handler));
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

    /// Public because modality and ownership by the main window are acceptance criteria.
    pub fn detached_window(&self) -> Option<adw::Window> {
        self.imp().detached.borrow().clone()
    }

    /// A no-op when nothing is being composed: there is no empty composer to detach.
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

    /// The same widget reparented, so every field, the identity override, undo history and cursor
    /// survive without a list of things to copy.
    pub fn detach(&self) {
        if self.is_detached() || !self.is_open() {
            return;
        }
        let Some(holder) = self.host() else {
            return;
        };
        let field = self.focused_field();

        // The main window goes all the way back first: the message returns to the reading pane and
        // the keyboard context leaves `Composer`.
        self.release_pane();

        let host = adw::Window::builder()
            .title(self.window_title())
            .transient_for(&holder.parent().unwrap_or_default())
            // Not modal: the point of detaching is to read something else while writing.
            .modal(false)
            .default_width(DETACHED_SIZE.0)
            .default_height(DETACHED_SIZE.1)
            .build();

        // A real header bar: `AdwWindow` draws no titlebar, so the pop-out would have no title,
        // close button or drag handle.
        holder.remove(self);
        match holder.frame() {
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

        // A forwarder to the main window's resolver, so `[keys]` reaches both containers and there
        // is one keymap.
        let keys = gtk::EventControllerKey::new();
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| composer.handle_key(key, state)
        ));
        host.add_controller(keys);

        host.connect_close_request(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_| {
                composer.close();
                // `close` has already destroyed the window; the default handler would tear down one
                // that is gone.
                glib::Propagation::Stop
            }
        ));

        self.imp().detached.replace(Some(host.clone()));
        sync_detach_button(&self.imp().detach, true);
        host.present();
        self.restore_focus(field);
    }

    /// The subject: a detached window must be identifiable without focus, and two replies would
    /// both say "Reply". The kind is the fallback with no subject.
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

    /// Unparenting drops focus, the one thing moving the composition loses. Falls back to where a
    /// fresh composition starts.
    fn restore_focus(&self, field: Option<Field>) {
        match field {
            Some(field) => {
                self.widget_for(field).grab_focus();
            }
            None => self.focus_first(),
        }
    }

    /// Split out because closing a detached composer must not retake the pane, which `attach` does.
    fn reclaim(&self) -> bool {
        let Some(host) = self.imp().detached.take() else {
            return false;
        };
        // Out of the toolbar view, not off the window: unparenting from elsewhere would leave GTK
        // holding a missing child.
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
        // Dispatched by this composer, not the host's key handling, which broadcasts to every
        // subscriber: with several composers open `Send` would send every draft (ADR 0034).
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
                // The header button reaches this action whether or not the composer is open; `c`
                // reaches `dispatch` and stays a pure open.
                // Only the button doubles as the close.
                if let Some(host) = composer.detached_window() {
                    // Detached, the button puts the composition's window in front.
                    host.present();
                } else if composer.is_open() {
                    composer.close();
                } else {
                    composer.open_new_draft();
                }
            }
        ));
        host.add_action(&action);

        // The broadcast belongs to whichever composer has the pane; a detached one hears its own
        // keys, and with two open `Send` would send both drafts (ADR 0034).
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

    /// A host that keeps the composer's widget alive and is kept alive by it is a reference cycle;
    /// whoever owns the host calls this on teardown.
    pub fn unmount(&self) {
        self.imp().host.take();
    }

    fn host(&self) -> Option<Rc<dyn ComposerHost>> {
        self.imp().host.borrow().clone()
    }

    /// Public so a host that resolves its own keys can hand the composer the command they ran.
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
            // #426: replacing an in-progress draft with a fresh reply would lose typed prose with
            // no confirmation; the refusal gets a status line instead of silence.
            CommandId::Reply | CommandId::ReplyAll | CommandId::Forward => {
                self.set_status(REPLY_BLOCKED);
            }
            CommandId::ToggleMarkdownSource if self.is_open() => self.toggle_markdown_source(),
            CommandId::AttachRemoteImage if self.is_open() => self.attach_remote(None),
            id if self.is_open() && self.imp().source.is_shown() && self.edit_source(id) => {}
            id if self.is_open() && WAITS_FOR_MARKDOWN.contains(&id) => {
                let keymap = self.keymap_now();
                let key = hints::key(&keymap, CommandId::ToggleMarkdownSource).unwrap_or_default();
                let words = postio_ui::compose::in_markdown_view_for_now(
                    postio_core::registry::get(id).title,
                    &key,
                );
                self.say_under_editor(&words);
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

    fn build_remote_rows(&self) -> gtk::Box {
        let imp = self.imp();
        let title = gtk::Label::new(Some(postio_ui::compose::NOT_ATTACHED_YET));
        title.add_css_class("postio-compose-label");
        title.set_xalign(0.0);
        imp.remote_box.append(&title);
        imp.remote_list.add_css_class("postio-compose-attachments");
        imp.remote_list.set_selection_mode(gtk::SelectionMode::None);
        imp.remote_box.append(&imp.remote_list);
        imp.remote_box.set_visible(false);
        imp.remote_box.clone()
    }

    fn render_remote_rows(&self) {
        let imp = self.imp();
        let images = self.document().remote_images();
        let states = imp.remote_state.borrow().clone();
        let drawn: Vec<_> = images
            .iter()
            .map(|(href, alt)| {
                (
                    href.as_str().to_owned(),
                    alt.clone(),
                    states.get(href.as_str()).cloned(),
                )
            })
            .collect();
        let key = hints::key(&self.keymap_now(), CommandId::AttachRemoteImage).unwrap_or_default();
        let waiting = images
            .iter()
            .filter(|(href, _)| states.get(href.as_str()) != Some(&RemoteState::Downloading))
            .count();
        match postio_ui::compose::remote_images_warning(waiting, &key) {
            Some(words) => {
                imp.remote_warning.set_label(&words);
                imp.remote_warning.set_visible(true);
            }
            None => imp.remote_warning.set_visible(false),
        }
        if *imp.remote_drawn.borrow() == drawn {
            return;
        }
        while let Some(row) = imp.remote_list.row_at_index(0) {
            imp.remote_list.remove(&row);
        }
        for (href, alt, state) in &drawn {
            imp.remote_list
                .append(&self.build_remote_row(href, alt, state.as_ref(), &key));
        }
        imp.remote_box.set_visible(!drawn.is_empty());
        imp.remote_drawn.replace(drawn);
    }

    fn build_remote_row(
        &self,
        href: &str,
        alt: &str,
        state: Option<&RemoteState>,
        key: &str,
    ) -> gtk::Widget {
        let host = postio_ui::compose::remote_host(href);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, crate::widgets::space::S3);
        row.set_margin_top(crate::widgets::space::S1);
        row.set_margin_bottom(crate::widgets::space::S1);
        let words = if alt.is_empty() { "image" } else { alt };
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "Image not attached: {words}, from {host}"
        ))]);
        let name = gtk::Label::new(Some(words));
        name.set_xalign(0.0);
        name.set_ellipsize(pango::EllipsizeMode::End);
        let meta = gtk::Label::new(Some(&match state {
            Some(RemoteState::Downloading) => postio_ui::compose::DOWNLOADING.to_owned(),
            Some(RemoteState::Refused(words)) => words.clone(),
            None => host.clone(),
        }));
        meta.add_css_class("postio-compose-label");
        meta.add_css_class("dim-label");
        meta.set_hexpand(true);
        meta.set_xalign(0.0);
        meta.set_wrap(true);
        let download = gtk::Button::new();
        download.set_child(Some(&keyhint::labelled(
            postio_ui::compose::DOWNLOAD_AND_ATTACH,
            (!key.is_empty()).then_some(key),
        )));
        download.add_css_class("postio-compose-remote-download");
        download.set_sensitive(state != Some(&RemoteState::Downloading));
        let target = postio_body::WebHref::parse(href);
        download.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[strong]
            target,
            move |_| composer.attach_remote(target.clone())
        ));
        let remove =
            crate::widgets::icon_button("edit-delete-symbolic", &format!("Send {words} as a link"));
        remove.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            #[strong]
            target,
            move |_| {
                if let Some(target) = target.as_ref() {
                    composer.unlink_remote(target);
                }
            }
        ));
        row.append(&name);
        row.append(&meta);
        row.append(&download);
        row.append(&remove);
        row.upcast()
    }

    fn attach_remote(&self, href: Option<postio_body::WebHref>) {
        let imp = self.imp();
        let images = self.document().remote_images();
        let target = href.or_else(|| {
            images.iter().map(|(href, _)| href.clone()).find(|href| {
                imp.remote_state.borrow().get(href.as_str()) != Some(&RemoteState::Downloading)
            })
        });
        let Some(target) = target else {
            self.say_under_editor(postio_ui::compose::NO_REMOTE_IMAGE);
            return;
        };
        let handler = imp.remote_image.borrow();
        let Some(handler) = handler.as_ref() else {
            self.say_under_editor("Downloading images is not available here.");
            return;
        };
        imp.remote_state
            .borrow_mut()
            .insert(target.as_str().to_owned(), RemoteState::Downloading);
        self.render_remote_rows();
        let draft = imp.draft.borrow().id;
        let weak = self.downgrade();
        let href = target.clone();
        handler(
            draft,
            target,
            Box::new(move |outcome| {
                if let Some(composer) = weak.upgrade() {
                    composer.remote_settled(&href, outcome);
                }
            }),
        );
    }

    fn remote_settled(
        &self,
        href: &postio_body::WebHref,
        outcome: Result<Attachment, postio_ui::compose::RemoteRefusal>,
    ) {
        let imp = self.imp();
        match outcome {
            Ok(attachment) => {
                imp.remote_state.borrow_mut().remove(href.as_str());
                let Some(content_id) = attachment
                    .content_id
                    .as_deref()
                    .and_then(postio_body::ContentId::parse)
                else {
                    return;
                };
                if imp.source.is_shown() {
                    let text = imp.source.text();
                    let from = format!("]({})", href.as_str());
                    if let Some(at) = text.find(&from) {
                        let to = format!("](cid:{})", content_id.as_str());
                        let mut edited = text.clone();
                        edited.replace_range(at..at + from.len(), &to);
                        let caret = imp.source.selection();
                        imp.source.apply(postio_ui::editor::markdown_edit::Edit {
                            text: edited,
                            selection: caret,
                        });
                    }
                } else {
                    let mut document = imp.body.document();
                    if document.attach_remote(href, &content_id) {
                        imp.body.show_at(document, None);
                    }
                }
                imp.draft.borrow_mut().attachments.push(attachment);
                self.render_attachments();
            }
            Err(refusal) => {
                let key = hints::key(&self.keymap_now(), CommandId::AttachRemoteImage)
                    .unwrap_or_default();
                let words = postio_ui::compose::remote_refusal(
                    &refusal,
                    &postio_ui::compose::remote_host(href.as_str()),
                    &key,
                );
                imp.remote_state
                    .borrow_mut()
                    .insert(href.as_str().to_owned(), RemoteState::Refused(words));
            }
        }
        self.refresh();
    }

    fn unlink_remote(&self, href: &postio_body::WebHref) {
        let imp = self.imp();
        imp.remote_state.borrow_mut().remove(href.as_str());
        if imp.source.is_shown() {
            let text = imp.source.text();
            let from = format!("]({})", href.as_str());
            if let Some(at) = text.find(&from)
                && let Some(bang) = text[..at].rfind("![")
            {
                let mut edited = text.clone();
                edited.remove(bang);
                let caret = imp.source.selection();
                imp.source.apply(postio_ui::editor::markdown_edit::Edit {
                    text: edited,
                    selection: caret.start.min(text.len() - 1)..caret.end.min(text.len() - 1),
                });
            }
        } else {
            let mut document = imp.body.document();
            if document.unlink_remote(href) {
                imp.body.show_at(document, None);
            }
        }
        self.refresh();
    }

    pub fn not_attached(&self) -> usize {
        self.document().remote_images().len()
    }

    #[doc(hidden)]
    pub fn test_remote_rows(&self) -> Vec<(String, String)> {
        let imp = self.imp();
        let states = imp.remote_state.borrow();
        self.document()
            .remote_images()
            .into_iter()
            .map(|(href, alt)| {
                let words = match states.get(href.as_str()) {
                    Some(RemoteState::Downloading) => postio_ui::compose::DOWNLOADING.to_owned(),
                    Some(RemoteState::Refused(words)) => words.clone(),
                    None => postio_ui::compose::remote_host(href.as_str()),
                };
                (alt, words)
            })
            .collect()
    }

    #[doc(hidden)]
    pub fn test_remote_warning(&self) -> Option<String> {
        let label = &self.imp().remote_warning;
        label.is_visible().then(|| label.label().to_string())
    }

    #[doc(hidden)]
    pub fn test_unlink_remote(&self, nth: usize) {
        if let Some((href, _)) = self.document().remote_images().get(nth) {
            self.unlink_remote(href);
        }
    }

    pub fn source_text(&self) -> Option<String> {
        let source = &self.imp().source;
        source.is_shown().then(|| source.text())
    }

    fn keymap_now(&self) -> Keymap {
        self.host()
            .map_or_else(|| Keymap::defaults().clone(), |host| host.keymap())
    }

    fn toggle_markdown_source(&self) {
        if self.imp().source.is_shown() {
            self.close_source();
            return;
        }
        // The caret first: the rich view says where it is asynchronously.
        let weak = self.downgrade();
        self.imp().body.caret(move |caret| {
            if let Some(composer) = weak.upgrade()
                && !composer.imp().source.is_shown()
            {
                let opened = postio_ui::editor::source::open(
                    &composer.imp().body.document(),
                    caret.as_ref(),
                );
                composer.show_source(&opened.text, opened.quotes, opened.caret);
            }
        });
    }

    fn show_source(&self, text: &str, quotes: Vec<postio_body::Block>, caret: Option<usize>) {
        let imp = self.imp();
        let keymap = self.keymap_now();
        let key = hints::key(&keymap, CommandId::ToggleMarkdownSource).unwrap_or_default();
        let narrow = self.width() > 0 && self.width() < NARROW_SOURCE;
        imp.source.show(
            text,
            quotes,
            caret,
            &postio_ui::compose::source_strip(&key, narrow),
        );
        imp.body.widget().set_visible(false);
        imp.source_toggle.set_active(true);
        self.hide_source_notice();
        imp.source.text_view().grab_focus();
    }

    fn close_source(&self) {
        let imp = self.imp();
        let text = imp.source.text();
        let caret = imp.source.selection().end;
        let returned = postio_ui::editor::source::close(&text, &imp.source.quotes(), Some(caret));
        self.hide_source();
        // A document the view only rewrote is the same document (FR-024).
        let before = imp.body.document();
        let room = postio_ui::editor::source::typing_room(&before);
        let unroomed = postio_body::Document {
            blocks: before.blocks[room..].to_vec(),
        };
        let document = if postio_body::markdown::normalize(unroomed) == returned.document {
            before
        } else {
            returned.document
        };
        imp.body.show_at(document, returned.caret.as_ref());
        match postio_ui::compose::narrowed_notice(returned.lost) {
            Some(words) => {
                imp.source_notice_label.set_label(&words);
                imp.source_notice_undo.set_visible(true);
                imp.source_notice.set_visible(true);
                imp.source_typed.replace(Some(text));
            }
            None => self.hide_source_notice(),
        }
        imp.body.widget().grab_focus();
        self.refresh();
    }

    /// Also on the line under the editor, since a framed host hides the status. Lasts until the
    /// next edit.
    fn say_under_editor(&self, words: &str) {
        self.set_status(words);
        let imp = self.imp();
        imp.source_typed.take();
        imp.source_notice_label.set_label(words);
        imp.source_notice_undo.set_visible(false);
        imp.source_notice.set_visible(true);
    }

    fn hide_source(&self) {
        let imp = self.imp();
        imp.source.hide();
        imp.body.widget().set_visible(true);
        imp.source_toggle.set_active(false);
    }

    fn hide_source_notice(&self) {
        let imp = self.imp();
        imp.source_notice.set_visible(false);
        imp.source_typed.take();
    }

    fn undo_narrowing(&self) {
        let imp = self.imp();
        let Some(typed) = imp.source_typed.take() else {
            return;
        };
        imp.body.undo();
        let quotes = postio_ui::editor::source::open(&imp.body.document(), None).quotes;
        self.show_source(&typed, quotes, Some(typed.len()));
        self.refresh();
    }

    fn build_source_notice(&self) -> gtk::Box {
        let imp = self.imp();
        imp.source_notice
            .add_css_class("postio-compose-source-notice");
        imp.source_notice.set_visible(false);
        imp.source_notice_label.set_xalign(0.0);
        imp.source_notice_label.set_wrap(true);
        imp.source_notice_label.set_hexpand(true);
        imp.source_notice
            .set_accessible_role(gtk::AccessibleRole::Status);
        imp.source_notice.append(&imp.source_notice_label);
        imp.source_notice_undo.add_css_class("flat");
        imp.source_notice_undo.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.undo_narrowing()
        ));
        imp.source_notice.append(&imp.source_notice_undo);
        imp.source_notice.clone()
    }

    fn edit_source(&self, id: CommandId) -> bool {
        use postio_ui::editor::markdown_edit as edit;
        let imp = self.imp();
        let text = imp.source.text();
        let at = imp.source.selection();
        let heading = |level: u8| edit::heading(&text, at.clone(), level);
        let result = match id {
            CommandId::Bold => edit::wrap(&text, at, "**", "**"),
            CommandId::Italic => edit::wrap(&text, at, "*", "*"),
            CommandId::Strikethrough => edit::strikethrough(&text, at),
            CommandId::InlineCode => edit::inline_code(&text, at),
            CommandId::InsertLink => edit::link(&text, at),
            CommandId::Heading1 => heading(1),
            CommandId::Heading2 => heading(2),
            CommandId::Heading3 => heading(3),
            CommandId::Heading4 => heading(4),
            CommandId::Heading5 => heading(5),
            CommandId::Heading6 => heading(6),
            CommandId::NormalText => edit::normal_text(&text, at),
            CommandId::BulletList => edit::toggle_prefix(&text, at, "- "),
            CommandId::NumberedList => edit::toggle_prefix(&text, at, "1. "),
            CommandId::QuoteBlock => edit::toggle_prefix(&text, at, "> "),
            CommandId::CodeBlock => edit::code_block(&text, at),
            CommandId::TaskList => edit::task_list(&text, at),
            CommandId::ToggleTask => edit::toggle_task(&text, at),
            CommandId::Indent => edit::indent(&text, at),
            CommandId::Outdent => edit::outdent(&text, at),
            CommandId::Divider => edit::divider(&text, at),
            CommandId::Table => edit::table(&text, at),
            CommandId::ClearFormatting => edit::clear_formatting(&text, at),
            _ => return false,
        };
        imp.source.apply(result);
        true
    }

    /// Entry-first and Enter-to-confirm: the hands are on the keyboard by construction.
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

    /// Does nothing without a source or when it has nothing to offer: `e` with no message open is
    /// not an error.
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
        // `identity_id` against whatever was last set, so a reply to account B sent as account A's
        // identity (#189).
        self.set_identities(account.identities.clone());
        self.set_signatures(account.signatures.clone());
        if let Some(draft) = reply_draft(id, &source, &account) {
            // After `open`, which clears the notice for every fresh composition.
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

    /// Not the draft cell's first: a reply fills the cell with the answered message's account,
    /// which changed where every later new message came from.
    fn account(&self) -> AccountId {
        let configured = self.imp().composing_from.get();
        if configured == AccountId::UNASSIGNED {
            self.imp().draft.borrow().account_id
        } else {
            configured
        }
    }

    /// Meant to be called once before the first composition: it writes onto the current draft, so
    /// mid-compose it would reassign what is being written.
    pub fn set_account(&self, account_id: AccountId) {
        self.imp().composing_from.set(account_id);
        self.imp().draft.borrow_mut().account_id = account_id;
    }

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

    /// The frame's controls call `send`, `send_later` and `dispatch`, so nothing about a
    /// composition changes with where buttons are drawn.
    /// The paperclip and "Attach another" hint go too: one control for one verb.
    pub fn set_framed(&self, framed: bool) {
        let imp = self.imp();
        for widget in imp.chrome.borrow().iter() {
            widget.set_visible(!framed);
        }
        imp.attach_button.set_visible(!framed);
        imp.attach_hint.set_visible(!framed);
    }

    /// Colours are read from probes wearing the reader's `.postio-flow-*` classes, so none is
    /// written in Rust and the body follows the host's light and dark.
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

    /// Part of what it opened with, not an edit: nothing is saved until the person writes
    /// something.
    pub fn start_with_labels(&self, labels: Vec<postio_model::LabelId>) {
        let imp = self.imp();
        imp.draft.borrow_mut().labels = labels;
        let was_filling = imp.filling.replace(true);
        self.refresh();
        imp.filling.set(was_filling);
    }

    /// A different verb from `open_file_chooser` (FR-049): a file chosen here ends up inside the
    /// message, one chosen there beside it.
    /// The bytes take the same path as a paste or drop.
    fn open_image_chooser(&self) {
        let Some(window) = self.host().and_then(|host| host.parent()) else {
            return;
        };
        // Unnamed on purpose: `FileFilter::set_name` would make `check-uncalled-pub-fn` match an
        // unrelated `set_name` in `postio-storage`.
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
        // From the file, not the extension: a `.png` that is a JPEG would be declared wrongly to
        // the recipient.
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

    /// Counts as an edit, so autosave and `changed` handlers see it like a typed word.
    fn add_attachment(&self, attachment: Attachment) {
        self.imp().draft.borrow_mut().attachments.push(attachment);
        self.render_attachments();
        self.refresh();
    }

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

    /// The draft's list is the single source of truth, never the widgets.
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
        let document = document_of(&draft.body);
        // A different draft is a different history: `load` clears the Editor's so undo cannot cross
        // the swap.
        imp.held.take();
        imp.remote_state.borrow_mut().clear();
        let quotes = postio_ui::editor::source::open(&document, None).quotes;
        imp.body.load(document);
        self.hide_source_notice();
        match draft.body_markdown.as_deref() {
            Some(typed) => {
                let mut text = typed.to_owned();
                for _ in &quotes {
                    text.push_str("\n\n");
                    text.push_str(&postio_ui::editor::source::quote_line());
                }
                self.show_source(&text, quotes, Some(typed.len()));
            }
            None => self.hide_source(),
        }
        imp.heading.set_text(heading(draft.kind));

        imp.cc_row.set_visible(!draft.cc.is_empty());
        imp.bcc_row.set_visible(!draft.bcc.is_empty());
        self.sync_more();
        // Ephemeral to one reply action: `open_reply` sets it right after this returns.
        self.set_tracking_domains(&[]);

        let identity = draft.identity_id;
        *imp.draft.borrow_mut() = draft;
        imp.downloading.borrow_mut().clear();
        imp.filling.set(false);
        self.render_attachments();

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

    /// A widget not yet mapped cannot take focus, so a failed grab retries after two tick
    /// callbacks: `idle_add_local_once` is not ordered against the frame clock that does the
    /// mapping and could lose the race (`postio-43`).
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

    /// Raising always works; putting away only while both fields are empty, since a hidden row
    /// holding addresses would still send to them. When refused it takes the keyboard to `Cc` so
    /// the refusal is visible.
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
            // The keyboard cannot be left in a hidden row, or Tab resumes from somewhere invisible.
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
        self.render_remote_rows();

        // A problem outranks a count; with no problem the line carries the recipient tally
        // (FR-023).
        match recipient_warning(&draft).or_else(|| recipient_summary(&draft)) {
            Some(text) => {
                imp.warning.set_text(&text);
                imp.warning.set_visible(true);
            }
            None => imp.warning.set_visible(false),
        }
        imp.send.set_sensitive(draft.is_sendable());

        // The subject is usually typed after detaching, and the title tells windows apart in the
        // list (FR-014).
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

    /// Re-arming cancels the pending source: a source id is not `Clone`.
    fn schedule_autosave(&self) {
        // Not mounted yet: this is `build()`'s own initial layout, not a user edit, and a save has
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
        // Editing undo is the document's and mail undo is `u`: the two-stacks separation of ADR
        // 0004 Q5, enforced inside the Editor.
        imp.body.connect_changed(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.body_edited()
        ));
        imp.source
            .text_view()
            .buffer()
            .connect_changed(glib::clone!(
                #[weak(rename_to = composer)]
                self,
                move |_| composer.body_edited()
            ));

        // At CAPTURE phase on purpose: WebKit keeps its own undo on a contenteditable surface, and
        // letting the keystroke through would run two histories against one document.
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
                    // A paste whose clipboard holds pixels is ours: WebKit would write an
                    // unresolvable fake URL into the DOM (#341).
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
        column.append(imp.source.widget());
        column.append(&self.build_source_notice());
        column.append(&imp.below_body);
        column.append(&imp.warning);
        column.append(&self.build_remote_rows());
        column.append(&self.build_attachments());
        imp.remote_warning.add_css_class("postio-compose-warning");
        imp.remote_warning.set_xalign(0.0);
        imp.remote_warning.set_wrap(true);
        imp.remote_warning.set_visible(false);
        imp.remote_warning
            .set_accessible_role(gtk::AccessibleRole::Status);
        column.append(&imp.remote_warning);
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
    /// destroy what the command acts on.
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

        // Next to the link button: both put something into the text (FR-049).
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

        // Through `dispatch` like every other button, so the control and the key cannot mean
        // different things (#1197).
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

        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        toolbar.append(&spacer);
        style_toolbar_button(
            imp.source_toggle.upcast_ref(),
            CommandId::ToggleMarkdownSource,
            "text-x-generic-symbolic",
            "postio-toolbar-markdown",
        );
        imp.source_toggle.connect_clicked(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |_| composer.dispatch(CommandId::ToggleMarkdownSource)
        ));
        toolbar.append(&imp.source_toggle);

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
        // The same verb the keyboard and palette reach, not a second implementation to keep in
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

    /// Discard is deliberately not here: a one-click destructive button beside Send is a misclick;
    /// `ctrl+d` and the palette reach it and both ask first.
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
        // Rebuilt every time the picker opens: the presets are relative to the moment it opens.
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

    /// A command whose binding the user cleared loses its hint rather than showing a blank. A hint
    /// that lies is worse than none (#828).
    pub fn set_keymap(&self, keymap: &Keymap) {
        let imp = self.imp();
        // One at a time: `schedule_send` is a `MenuButton` and the other two are `Button`s.
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

        let escape = hints::hint(keymap, CommandId::Back, "keeps the draft");
        imp.escape.set_label(&hints::line(escape.iter()));
        imp.escape.set_visible(escape.is_some());
    }

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

    #[doc(hidden)]
    pub fn test_source_text(&self) -> Option<String> {
        self.source_text()
    }

    #[doc(hidden)]
    pub fn test_source_caret(&self) -> usize {
        self.imp().source.selection().end
    }

    #[doc(hidden)]
    pub fn test_source_select(&self, from: usize, to: usize) {
        self.imp().source.select(from..to);
    }

    #[doc(hidden)]
    pub fn test_source_key(&self, key: gdk::Key) {
        self.imp().source.test_key(key);
    }

    #[doc(hidden)]
    pub fn test_source_undo(&self) {
        self.imp().source.undo();
    }

    #[doc(hidden)]
    pub fn test_source_paste(&self, text: &str) {
        self.imp().source.paste(text);
    }

    #[doc(hidden)]
    pub fn test_source_set(&self, text: &str) {
        self.imp().source.set_text(text);
    }

    #[doc(hidden)]
    pub fn test_source_strip(&self) -> Option<String> {
        let source = &self.imp().source;
        source.is_shown().then(|| source.strip())
    }

    #[doc(hidden)]
    pub fn test_source_notice(&self) -> Option<String> {
        let imp = self.imp();
        imp.source_notice
            .is_visible()
            .then(|| imp.source_notice_label.label().to_string())
    }

    #[doc(hidden)]
    pub fn test_press_source_notice_undo(&self) {
        self.imp().source_notice_undo.emit_clicked();
    }

    #[doc(hidden)]
    pub fn test_caret(&self) -> Option<postio_body::markdown::DocPosition> {
        self.imp().body.test_caret()
    }

    #[doc(hidden)]
    pub fn test_can_undo(&self) -> bool {
        self.imp().body.can_undo()
    }

    #[doc(hidden)]
    pub fn test_undo(&self) {
        self.undo_edit();
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

/// The formatting commands only the Markdown view makes for now: in the rich view each says where
/// it is, never nothing (FR-029).
const WAITS_FOR_MARKDOWN: [CommandId; 16] = [
    CommandId::Heading2,
    CommandId::Heading3,
    CommandId::Heading4,
    CommandId::Heading5,
    CommandId::Heading6,
    CommandId::NormalText,
    CommandId::Strikethrough,
    CommandId::InlineCode,
    CommandId::CodeBlock,
    CommandId::TaskList,
    CommandId::ToggleTask,
    CommandId::Indent,
    CommandId::Outdent,
    CommandId::Divider,
    CommandId::Table,
    CommandId::ClearFormatting,
];

const NARROW_SOURCE: i32 = 760;

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

/// Title and binding both read from the registry, so the toolbar cannot drift from the palette and
/// cheat sheet.
fn style_toolbar_button(button: &gtk::Button, id: CommandId, icon: &str, class: &str) {
    let spec = postio_core::registry::get(id);
    button.set_icon_name(icon);
    crate::widgets::button::dress_icon(button, spec.title);
    button.add_css_class(class);
    button.set_tooltip_text(Some(&format!(
        "{} ({})",
        spec.title,
        // Expanded first: the registry stores `mod+shift+8`, which a tooltip cannot show.
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

/// The type comes from the shared-mime-info sniff, not the extension; past the size cap the file is
/// attached, not inlined.
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

fn action_hints(keymap: &Keymap) -> Vec<(CommandId, Option<String>)> {
    ACTION_BUTTONS
        .iter()
        .map(|(id, _)| (*id, keymap.binding(*id).map(str::to_owned)))
        .collect()
}

/// One control rather than two: a button correct only while the composer is in one of its two
/// places lies half the time.
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
    /// A hide waits for the main loop so a change the next one undoes (`set_text`'s
    /// empty-then-filled, a paste) leaves the popover up:
    /// on Wayland a repositioned-before-drawn popup waits on a reply the compositor never sends,
    /// and GTK blocks.
    hiding: Rc<RefCell<Option<glib::SourceId>>>,
    /// A popup shown a frame after it hid reuses its surface while the renderer may still have a
    /// frame in flight;
    /// the compositor drops the client ("already has a buffer committed") or GTK waits forever.
    hidden_at: Rc<Cell<Option<std::time::Instant>>>,
    showing: Rc<RefCell<Option<glib::SourceId>>>,
}

const REAPPEAR_AFTER: std::time::Duration = std::time::Duration::from_millis(150);

impl Completion {
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
        // A custom child: GTK requires unparenting before the entry is finalized, or the popover
        // and list outlive the composer.
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

        // A click commits the row it landed on (#424).
        this.list.connect_row_activated({
            // The list belongs to Completion: a strong clone would keep the popover alive after the
            // composer is destroyed.
            let this = Rc::downgrade(&this);
            let entry = entry.downgrade();
            move |_, row| {
                if let (Some(this), Some(entry)) = (this.upgrade(), entry.upgrade()) {
                    this.accept_row(&entry, row);
                }
            }
        });

        let keys = gtk::EventControllerKey::new();
        // Capture phase: the focused `GtkText` binds Return to `activate` and consumes it before a
        // bubble-phase controller sees it (#424).
        // `handle_key` declines every key while the popover is closed.
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
        // `chars`, since bytes would ask more of one script than another.
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

    /// Takes the row the click hit rather than re-reading the selection, so a click can never
    /// commit something other than what was under the pointer.
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

/// HTML is the record (ADR 0004 Q3), so a draft that has one is parsed from it; the text half
/// covers older drafts and plain messages.
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
        // Asserting `Fill` against a function that returns nothing else proves nothing; the
        // behaviour (#691) is asserted on the widget in
        // `gtk_composer_resume::composing_after_a_kept_draft_starts_blank`.
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
        // #456: a message sent as plain text format=flowed whose sentence wraps across three lines
        // must quote as the one sentence.
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
        // A message with short lines and no format=flowed must not be reflowed: intentional breaks
        // would be eaten.
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
        // It has to be `save_draft` that loses the key: most verbs carry an alternate and cannot be
        // emptied, but `save_draft` has one key.
        // An explicit `[keys]` entry outranks a default.
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
