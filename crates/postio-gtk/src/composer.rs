//! The composer: canvas 2a, taking over the reading pane.
//!
//! # Why it is not a window
//!
//! `spec.md` §10 implies a separate compose window; canvas 2a is explicit that
//! it is not, and where the two disagree the canvas wins. The composer
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
//! * Sending through the operation queue is `postio-pzy`, autosave is
//!   `postio-own`, attachments `postio-tws`, recipient completion `postio-agd`,
//!   and reply seeding and quoting `postio-p8q` — which builds the [`Draft`]
//!   that [`Composer::open`] takes.
//!
//! Until a handler is connected the composer says so instead of pretending:
//! [`CommandId::Send`] with nothing listening leaves the draft in place and
//! names the missing piece on the status line, because a composer that empties
//! itself into a seam that is not connected yet has silently lost the mail.
//!
//! [`connect_send`]: Composer::connect_send

use std::cell::{Cell, RefCell};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::{CommandId, Context};
use postio_model::address::{format_list, parse_list};
use postio_model::signature;
use postio_model::{AccountId, Draft, DraftKind, Identity, IdentityId, MessageBody};

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
const UNSAVED: &str = "draft is in the composer only";

/// What the status line says when opening compose came back to a kept draft.
const RETAINED: &str = "still holding the draft Esc kept";

/// What the status line says when [`CommandId::Send`] has nowhere to go.
const NO_SEND_PATH: &str = "not sent — no outgoing account is connected yet";

/// The class `shell.css` dims the sidebar and the list under.
///
/// Canvas 2a's own signal that the keyboard is in the composer: the list is
/// still there, still scrolled where it was, and visibly not what is being
/// typed into.
pub const COMPOSING_CLASS: &str = "composing";

/// What to call with the draft when the user sends or saves.
type DraftHandler = Box<dyn Fn(&Draft)>;

/// What to call when the composer closes, with what became of the draft.
type ClosedHandler = Box<dyn Fn(Closing)>;

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
        pub identity_row: gtk::Box,
        pub identity: gtk::DropDown,
        pub identity_only: gtk::Label,
        pub body: gtk::TextView,
        pub send: gtk::Button,
        pub save: gtk::Button,
        pub warning: gtk::Label,
        /// Everything about the draft that is not in a field: its id, account,
        /// kind, attachments, and what it is a reply to.
        pub draft: RefCell<Draft>,
        pub identities: RefCell<Vec<Identity>>,
        pub sent: RefCell<Vec<DraftHandler>>,
        pub saved: RefCell<Vec<DraftHandler>>,
        pub changed: RefCell<Vec<DraftHandler>>,
        pub closed: RefCell<Vec<ClosedHandler>>,
        /// The window the composer took a pane from, once mounted.
        pub window: glib::WeakRef<Window>,
        /// The context and pane to put back when the composer closes.
        pub restore: Cell<Option<(Context, Pane)>>,
        /// Set while `open` is filling the fields, so the widgets' own
        /// `changed` signals do not report the fill as the user typing.
        pub filling: Cell<bool>,
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
                identity_row: row(),
                identity: gtk::DropDown::from_strings(&[]),
                identity_only: gtk::Label::new(None),
                body: gtk::TextView::new(),
                send: gtk::Button::new(),
                save: gtk::Button::new(),
                warning: gtk::Label::new(None),
                draft: RefCell::new(Draft::new(AccountId::UNASSIGNED)),
                identities: RefCell::new(Vec::new()),
                sent: RefCell::new(Vec::new()),
                saved: RefCell::new(Vec::new()),
                changed: RefCell::new(Vec::new()),
                closed: RefCell::new(Vec::new()),
                window: glib::WeakRef::new(),
                restore: Cell::new(None),
                filling: Cell::new(false),
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
        // keyboard back, never a reset of what is being typed.
        if self.is_open() {
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
        let buffer = imp.body.buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        draft.body = MessageBody {
            text: (!text.is_empty()).then_some(text),
            html: draft.body.html,
        };
        draft
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
    pub fn save(&self) {
        let draft = self.draft();
        for handler in self.imp().saved.borrow().iter() {
            handler(&draft);
        }
    }

    /// Called with the draft when the user sends it.
    pub fn connect_send(&self, handler: impl Fn(&Draft) + 'static) {
        self.imp().sent.borrow_mut().push(Box::new(handler));
    }

    /// Called with the draft when the user asks for it to be saved.
    pub fn connect_save(&self, handler: impl Fn(&Draft) + 'static) {
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
            move |_, _| composer.open(Draft::new(composer.account()))
        ));
        window.add_action(&action);

        window.connect_command(glib::clone!(
            #[weak(rename_to = composer)]
            self,
            move |id| composer.dispatch(id)
        ));
    }

    /// Acts on the commands the composer owns, and ignores the rest.
    fn dispatch(&self, id: CommandId) {
        match id {
            CommandId::Compose => self.open(Draft::new(self.account())),
            CommandId::Send if self.is_open() => self.send(),
            CommandId::SaveDraft if self.is_open() => self.save(),
            CommandId::DiscardDraft if self.is_open() => self.request_discard(),
            CommandId::Back if self.is_open() => {
                self.close();
            }
            _ => {}
        }
    }

    fn account(&self) -> AccountId {
        self.imp().draft.borrow().account_id
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

        imp.to.set_text(&format_list(&draft.to));
        imp.cc.set_text(&format_list(&draft.cc));
        imp.bcc.set_text(&format_list(&draft.bcc));
        imp.subject.set_text(&draft.subject);
        imp.body
            .buffer()
            .set_text(draft.body.text.as_deref().unwrap_or_default());
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

        let title = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        title.add_css_class("postio-compose-title");
        title.append(&imp.heading);
        title.append(&imp.status);

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
        // Tab moves to the next control rather than typing a tab: the body is
        // in the middle of the focus chain, and a keyboard user has to be able
        // to get out of it.
        imp.body.set_accepts_tab(false);
        imp.body
            .update_property(&[gtk::accessible::Property::Label("Message body")]);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&imp.body)
            .build();

        imp.warning.add_css_class("postio-compose-warning");
        imp.warning.set_xalign(0.0);
        imp.warning.set_visible(false);
        imp.warning.set_accessible_role(gtk::AccessibleRole::Status);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&title);
        column.append(&fields);
        column.append(&scroller);
        column.append(&imp.warning);
        column.append(&self.build_actions());
        self.set_child(Some(&column));

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
}
