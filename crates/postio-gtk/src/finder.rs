//! One box: search mail, run a command, jump to a folder.
//!
//! # Why there is only one
//!
//! Postio used to have two surfaces for this — a `Ctrl+K` command palette and
//! a `/` query bar — and the user had to know which box a given thing lived
//! behind before they could reach it. That is exactly the friction the
//! product exists to remove, and search is already the primary way to move
//! around (docs/PRODUCT.md §7). So they are one box, VS Code's way: one keypress, one
//! field, and a prefix that says which question you are asking.
//!
//! # Where the box is
//!
//! In the header, where canvas 1b already draws it. That is not a
//! simplification of the canvas but a reading of it: 1b draws the field at
//! rest — *Search all mail*, with its `/` — and 2b draws the same field
//! active, outlined in steel with the query's operators as chips inside it.
//! Two drawings of one thing. The results for the modes that *have* results
//! drop below it on the plate the palette already used.
//!
//! # The modes
//!
//! | Typed | Mode | What it does |
//! |---|---|---|
//! | anything | [`Mode::Search`] | searches mail, operators become chips |
//! | `>` | [`Mode::Command`] | fuzzy-matches the command registry |
//! | `#` | [`Mode::Mailbox`] | jumps to a folder |
//! | `@` | [`Mode::Contact`] | finds a correspondent, and searches their mail |
//!
//! Every one of them answers the same question — *where is the thing I am
//! looking for* — which is why picking a contact searches their mail rather
//! than composing to them. Composing is `c`, a verb of its own with a surface
//! of its own; a mode that ended somewhere else would be a mode you could
//! fall out of. Picking a contact leaves the box in [`Mode::Search`] holding
//! `from:<address>`, which is an ordinary chip the user can then build on.
//!
//! A prefix typed into an empty box is *absorbed* — it leaves the text and
//! becomes the mode, shown as a marker in the field. That is what makes the
//! mode visible at a glance and Backspace-at-the-start reversible, and it
//! keeps the query the user is editing free of punctuation that is not
//! theirs.
//!
//! Results are ranked within the active mode and never blended. A list that
//! mixes commands and messages is one that is harder to scan than either.
//!
//! # Two halves
//!
//! [`Query`] and [`folders`] are pure and tested without a display; the
//! matching itself is [`crate::palette::score`], and the chips are
//! [`crate::search::chips`]. Neither engine changes — this is a convergence
//! of two surfaces, not of two parsers.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};
use postio_core::{ActionId, Context, Keymap, Scope};
use postio_model::Contact;
use postio_model::ids::MailboxId;
use postio_model::mailbox::Mailbox;
use postio_search::ParsedQuery;

use crate::palette::{Entry, entries, highlight, score};
use crate::search::{Backspace, Chip, Live, backspace, chips};

/// Which question the box is asking.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Search mail. The default, because it is the common case.
    #[default]
    Search,
    /// Run a command, fuzzy-matched over the registry.
    Command,
    /// Jump to a folder.
    Mailbox,
    /// Find a correspondent, and search their mail.
    Contact,
}

impl Mode {
    /// Every mode, in the order the cheat sheet should list them.
    pub const ALL: [Mode; 4] = [Mode::Search, Mode::Command, Mode::Mailbox, Mode::Contact];

    /// The character that switches into this mode from an empty box.
    ///
    /// `None` for [`Mode::Search`]: it is what the box already is, so there
    /// is nothing to type to get there.
    pub const fn prefix(self) -> Option<char> {
        match self {
            Mode::Search => None,
            Mode::Command => Some('>'),
            Mode::Mailbox => Some('#'),
            Mode::Contact => Some('@'),
        }
    }

    /// The mode a prefix character asks for.
    pub fn of_prefix(character: char) -> Option<Mode> {
        Mode::ALL
            .into_iter()
            .find(|mode| mode.prefix() == Some(character))
    }

    /// The marker drawn in the field, so the mode is visible at a glance.
    ///
    /// Search wears the `/` the canvas already draws on the field.
    pub const fn marker(self) -> &'static str {
        match self {
            Mode::Search => "/",
            Mode::Command => ">",
            Mode::Mailbox => "#",
            Mode::Contact => "@",
        }
    }

    /// What the empty box invites the user to do.
    pub const fn placeholder(self) -> &'static str {
        match self {
            Mode::Search => "Search all mail",
            Mode::Command => "Run a command",
            Mode::Mailbox => "Go to a folder",
            Mode::Contact => "Find a correspondent",
        }
    }

    /// The keyboard context this mode owns while the box is open.
    ///
    /// Two contexts rather than one because `Enter` means different things —
    /// run this command, or search for this — and `postio-core`'s registry
    /// already distinguishes them.
    pub const fn context(self) -> Context {
        match self {
            Mode::Search => Context::Search,
            Mode::Command | Mode::Mailbox | Mode::Contact => Context::Palette,
        }
    }

    /// Whether this mode answers with a list under the field.
    ///
    /// Search does not: its results are the message list itself, which is the
    /// whole reason search is navigation here rather than a dialog.
    pub const fn has_results(self) -> bool {
        !matches!(self, Mode::Search)
    }
}

/// What the box is asking, and what has been typed into it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    /// Which question.
    pub mode: Mode,
    /// The text after the marker. Never contains the prefix.
    pub text: String,
}

impl Query {
    /// An empty box, searching mail.
    pub fn new() -> Self {
        Self::default()
    }

    /// The box that starts in `mode` with nothing typed.
    pub fn in_mode(mode: Mode) -> Self {
        Query {
            mode,
            text: String::new(),
        }
    }

    /// Whether anything has been typed.
    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// What the box becomes when its entry now holds `text`.
    ///
    /// A prefix at the start of a search absorbs into the mode. Only out of
    /// [`Mode::Search`]: once a mode is chosen, a `>` is a character like any
    /// other, or `>` could never be searched for at all.
    pub fn typed(&self, text: &str) -> Query {
        if self.mode != Mode::Search {
            return Query {
                mode: self.mode,
                text: text.to_owned(),
            };
        }
        let mut characters = text.chars();
        match characters.next().and_then(Mode::of_prefix) {
            Some(mode) => Query {
                mode,
                text: characters.as_str().to_owned(),
            },
            None => Query {
                mode: Mode::Search,
                text: text.to_owned(),
            },
        }
    }

    /// What Backspace at the very start of the text does.
    ///
    /// `None` means the entry should handle it itself — there is nothing to
    /// back out of. Otherwise the mode is given up and whatever was typed
    /// stays, so backing out of a mode never costs the words.
    pub fn backspace_at_start(&self) -> Option<Query> {
        (self.mode != Mode::Search).then(|| Query {
            mode: Mode::Search,
            text: self.text.clone(),
        })
    }
}

/// One folder the box matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FolderHit {
    /// The folder to open.
    pub id: MailboxId,
    /// What the sidebar calls it — one word for one mailbox, everywhere.
    pub name: String,
    /// Unread, for the count beside the row.
    pub unread: u32,
    /// Byte indices in `name` the query matched, for highlighting.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// The folders matching `query`, best first.
///
/// Scored with the palette's own matcher, so `wd` finds `wayland-devel` in
/// this box exactly as `cp` finds "Command palette" in the other mode.
pub fn folders(mailboxes: &[Mailbox], query: &str) -> Vec<FolderHit> {
    let query = query.trim();
    let mut found: Vec<FolderHit> = mailboxes
        .iter()
        .filter(|mailbox| mailbox.selectable)
        .filter_map(|mailbox| {
            let name = crate::sidebar::display_name(mailbox);
            let matched = score(query, &name)?;
            Some(FolderHit {
                id: mailbox.id,
                name,
                unread: mailbox.counts.unread,
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();
    // Stable, so an empty query leaves the sidebar's own order alone.
    found.sort_by_key(|hit| std::cmp::Reverse(hit.score));
    found.truncate(crate::palette::MAX_ROWS);
    found
}

/// One correspondent the box matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ContactHit {
    /// What to call them: the name the user set, then the last name seen on
    /// the address, then the address itself.
    pub name: String,
    /// The addr-spec, which is what `from:` will be given.
    pub address: String,
    /// How many messages this address has been seen on, for the cap beside
    /// the row — a correspondent you write to daily reads differently from
    /// one who mailed you once.
    pub times_seen: u32,
    /// Byte indices in `name` the query matched, for highlighting.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// The correspondents matching `query`, best first.
///
/// Scored with the palette's own matcher over the *name*, and again over the
/// address when the name did not match — people look for `grace`, and they
/// look for `gh`, and they look for `@example.org`. One matcher across the
/// whole box, so `gh` finds Grace Hopper here exactly as `wd` finds
/// `wayland-devel` one mode over.
///
/// Ties break on how often the correspondent has been seen, which is the same
/// ranking `ContactRepository::search` uses and the reason an empty query
/// offers the people you actually write to.
pub fn contacts(contacts: &[Contact], query: &str) -> Vec<ContactHit> {
    let query = query.trim();
    let mut found: Vec<ContactHit> = contacts
        .iter()
        .filter_map(|contact| {
            let name = contact_name(contact);
            let address = contact.address.address.clone();
            // The name first, so the highlight lands on what the row shows.
            // Falling back to the address means `@example.org` still finds
            // people, and costs nothing when the name already matched.
            let matched = match score(query, &name) {
                Some(matched) => matched,
                None => score(query, &address).map(|matched| crate::palette::Match {
                    score: matched.score,
                    positions: Vec::new(),
                })?,
            };
            Some(ContactHit {
                name,
                address,
                times_seen: contact.times_seen,
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();
    found.sort_by_key(|hit| {
        (
            std::cmp::Reverse(hit.score),
            std::cmp::Reverse(hit.times_seen),
        )
    });
    found.truncate(crate::palette::MAX_ROWS);
    found
}

/// What to call a correspondent: the name the user set, then the last display
/// name seen on the address, then the address itself. Never empty, so a row
/// always has something to say.
fn contact_name(contact: &Contact) -> String {
    contact
        .name
        .clone()
        .or_else(|| contact.address.name.clone())
        .unwrap_or_else(|| contact.address.address.clone())
}

/// The query picking `hit` puts in the box.
///
/// A `from:` chip, quoted if the address could not survive being typed back
/// in. Deliberately *the query*, not a search that has already run: the point
/// of landing in [`Mode::Search`] is that the user can go on building on it.
pub fn contact_query(hit: &ContactHit) -> String {
    if hit.address.chars().any(char::is_whitespace) {
        format!("from:\"{}\"", hit.address.replace('"', ""))
    } else {
        format!("from:{}", hit.address)
    }
}

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

/// The header field the box lives in, and the parts of it the mode changes.
#[derive(Clone)]
pub struct Field {
    /// The field's own box, for the class that marks it active.
    pub frame: gtk::Box,
    /// The magnifier, shown while searching.
    pub icon: gtk::Image,
    /// The mode marker, shown instead of the magnifier otherwise.
    pub marker: gtk::Label,
    /// The text the user types into.
    pub text: gtk::Text,
    /// The live hit count and timing, at the right-hand end of the field.
    pub readout: gtk::Label,
    /// The `/` cap at the right, hidden once the box is open.
    pub hint: gtk::Label,
}

type CommandHandler = Box<dyn Fn(ActionId)>;
type FolderHandler = Box<dyn Fn(MailboxId)>;
type ContactHandler = Box<dyn Fn(&ContactHit)>;
type QueryHandler = Box<dyn Fn(&ParsedQuery)>;

mod imp {
    use std::cell::{Cell, RefCell};

    use super::*;

    pub struct Finder {
        pub(super) field: RefCell<Option<Field>>,
        /// The query's operators, read back as chips under the field.
        pub(super) chips: gtk::Box,
        pub(super) list: gtk::ListBox,
        pub(super) empty: gtk::Label,
        pub(super) scroller: gtk::ScrolledWindow,
        pub(super) keymap: RefCell<Keymap>,
        /// The workspace's context, for filtering which commands apply.
        pub(super) context: RefCell<Context>,
        /// What the mail on screen belongs to. Beside `context` because it
        /// answers the same question from the other side: `context` is which
        /// surface has focus, this is what that surface is showing, and a
        /// command can need either (#182).
        pub(super) scope: RefCell<Scope>,
        pub(super) mailboxes: RefCell<Vec<Mailbox>>,
        pub(super) contacts: RefCell<Vec<Contact>>,
        pub(super) query: RefCell<Query>,
        pub(super) parsed: RefCell<ParsedQuery>,
        /// The hit count and timing, once `attach` has a field to draw it in.
        pub(super) live: RefCell<Option<Live>>,
        pub(super) commands: RefCell<Vec<ActionId>>,
        pub(super) folders: RefCell<Vec<MailboxId>>,
        pub(super) matched: RefCell<Vec<ContactHit>>,
        pub(super) open: Cell<bool>,
        /// Set while the box is rewriting its own entry, so absorbing a
        /// prefix does not read as the user typing again.
        pub(super) echoing: Cell<bool>,
        pub(super) on_command: RefCell<Vec<CommandHandler>>,
        pub(super) on_folder: RefCell<Vec<FolderHandler>>,
        pub(super) on_contact: RefCell<Vec<ContactHandler>>,
        pub(super) on_search: RefCell<Vec<QueryHandler>>,
        pub(super) on_changed: RefCell<Vec<QueryHandler>>,
        pub(super) on_dismissed: RefCell<Vec<Box<dyn Fn()>>>,
        /// What `Tab` hands the keyboard to, if anything will take it.
        pub(super) on_tab: RefCell<Vec<Box<dyn Fn() -> bool>>>,
    }

    impl Default for Finder {
        fn default() -> Self {
            Finder {
                field: RefCell::new(None),
                chips: gtk::Box::new(gtk::Orientation::Horizontal, 6),
                list: gtk::ListBox::new(),
                empty: gtk::Label::new(None),
                scroller: gtk::ScrolledWindow::new(),
                keymap: RefCell::new(Keymap::default()),
                // The list is where the box opens from, and the context the
                // commands are filtered by until the window says otherwise.
                context: RefCell::new(Context::List),
                scope: RefCell::new(Scope::default()),
                mailboxes: RefCell::new(Vec::new()),
                contacts: RefCell::new(Vec::new()),
                query: RefCell::new(Query::new()),
                parsed: RefCell::new(ParsedQuery::default()),
                live: RefCell::new(None),
                commands: RefCell::new(Vec::new()),
                folders: RefCell::new(Vec::new()),
                matched: RefCell::new(Vec::new()),
                open: Cell::new(false),
                echoing: Cell::new(false),
                on_command: RefCell::new(Vec::new()),
                on_folder: RefCell::new(Vec::new()),
                on_contact: RefCell::new(Vec::new()),
                on_search: RefCell::new(Vec::new()),
                on_changed: RefCell::new(Vec::new()),
                on_dismissed: RefCell::new(Vec::new()),
                on_tab: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Finder {
        const NAME: &'static str = "PostioFinder";
        type Type = super::Finder;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Finder {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Finder {}
    impl BinImpl for Finder {}
}

glib::wrapper! {
    /// The results under the box: commands, or folders.
    ///
    /// The widget is only the plate. The box itself is the header's field,
    /// handed over by [`Finder::attach`] — there is one field in this
    /// application and it is the one the canvas draws.
    pub struct Finder(ObjectSubclass<imp::Finder>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Finder {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Finder {
    /// A closed box over the registry defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// Drive `field` — the header's search box.
    ///
    /// Called once. Everything the user types goes through here.
    pub fn attach(&self, field: &Field) {
        let imp = self.imp();
        *imp.field.borrow_mut() = Some(field.clone());
        *imp.live.borrow_mut() = Some(Live::new(field.readout.clone()));

        field.text.connect_changed(glib::clone!(
            #[weak(rename_to = finder)]
            self,
            move |text| {
                if finder.imp().echoing.get() {
                    return;
                }
                finder.retype(&text.text());
            }
        ));

        field.text.connect_activate(glib::clone!(
            #[weak(rename_to = finder)]
            self,
            move |_| finder.activate()
        ));

        // Focusing the field *is* opening the box: a user who clicks it has
        // asked the same question `/` asks. `begin` rather than `open`,
        // because `open` grabs the focus — and grabbing it from inside the
        // handler that fired *because* focus arrived is a loop the frame
        // clock never gets out of.
        //
        // Only when the box is *closed*, though. Every other mode opens by
        // grabbing the focus itself, so this handler fires a moment later for
        // a box that is already open — and answering it with `Mode::Search`
        // threw the mode away before the user could type into it. `>`, `#`
        // and `@` all landed back in search that way.
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[weak(rename_to = finder)]
            self,
            move |_| {
                if !finder.is_open() {
                    finder.begin(Mode::Search);
                }
            }
        ));
        field.text.add_controller(focus);

        // Capture, so Backspace is decided before the entry deletes a
        // character, and Escape before the entry swallows it.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = finder)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| match key {
                gtk::gdk::Key::BackSpace if finder.press_backspace() => glib::Propagation::Stop,
                gtk::gdk::Key::Tab if finder.press_tab() => glib::Propagation::Stop,
                gtk::gdk::Key::Escape => {
                    finder.dismiss();
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Up => {
                    finder.move_selection(-1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Down => {
                    finder.move_selection(1);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        ));
        field.text.add_controller(keys);

        self.render();
    }

    /// The bindings to show beside each command.
    pub fn set_keymap(&self, keymap: Keymap) {
        *self.imp().keymap.borrow_mut() = keymap;
        self.refresh();
    }

    /// The workspace context, which decides what commands apply.
    pub fn set_context(&self, context: Context) {
        *self.imp().context.borrow_mut() = context;
        self.refresh();
    }

    /// What the mail on screen belongs to, for the commands that need one
    /// account rather than one surface — `Move`, so far (#182).
    pub fn set_scope(&self, scope: Scope) {
        *self.imp().scope.borrow_mut() = scope;
        self.refresh();
    }

    /// The folders `#` can jump to.
    pub fn set_mailboxes(&self, mailboxes: &[Mailbox]) {
        *self.imp().mailboxes.borrow_mut() = mailboxes.to_vec();
        self.refresh();
    }

    /// The correspondents `@` can find.
    ///
    /// The whole list, not a prefix query: the matcher is a subsequence one,
    /// so `gh` has to be able to reach `Grace Hopper`, and no SQL `LIKE` will
    /// find that. A mailbox's distinct correspondents are bounded by the
    /// people who have written to it — thousands, not the millions of
    /// messages — and [`crate::palette::MAX_ROWS`] bounds what is drawn.
    pub fn set_contacts(&self, contacts: &[Contact]) {
        *self.imp().contacts.borrow_mut() = contacts.to_vec();
        self.refresh();
    }

    /// Whether the box has the keyboard.
    pub fn is_open(&self) -> bool {
        self.imp().open.get()
    }

    /// What the box is asking, and what is typed into it.
    pub fn query(&self) -> Query {
        self.imp().query.borrow().clone()
    }

    /// Which question the box is asking.
    pub fn mode(&self) -> Mode {
        self.imp().query.borrow().mode
    }

    /// The keyboard context while the box is open.
    pub fn context(&self) -> Option<Context> {
        self.is_open().then(|| self.mode().context())
    }

    /// Whether the box has the keyboard, as opposed to merely being open.
    ///
    /// These stopped being the same question the day search began leaving the
    /// field up after a query — "its results *are* the message list", so the
    /// box does not close itself and `is_open` stays true while the user goes
    /// back to reading mail. `Window::key_context` asked `is_open`, so from
    /// the first search onwards the resolver was pinned to `Search`, where
    /// `?` and the rest are not bound, and every bare key was silently
    /// dropped for the rest of the session. That is `postio-73`, reported as
    /// "single-key bindings stop working, seemingly at random" — random
    /// because nothing times out and nothing else clears it.
    ///
    /// So ownership of the keyboard is asked of the keyboard. The field is
    /// the box: there is one `GtkText` in this application and the header
    /// draws it, per [`Finder::attach`]. The `focus_child` arm covers the
    /// results plate, which can hold focus without the field doing so.
    ///
    /// `is_focus`, not `has_focus`: the question is which widget the keyboard
    /// is on *in this window*, and `has_focus` additionally requires the
    /// window to be the active one. Asking that would hand the bindings back
    /// to the message list every time the user alt-tabbed away with the
    /// search box focused, and would answer the same way under a headless
    /// compositor, where no window is ever active. `Window::is_typing` asks
    /// the window for its focus widget for the same reason.
    pub fn has_keyboard(&self) -> bool {
        self.imp()
            .field
            .borrow()
            .as_ref()
            .is_some_and(|field| field.text.is_focus())
            || self.focus_child().is_some()
    }

    /// The live hit count and timing, once the box has a field to draw in.
    ///
    /// This is where a search actually gets run from: whatever owns the store
    /// calls [`Live::connect_run`] and answers with [`Live::deliver`]. The box
    /// itself only knows how to pace the question, not how to answer it.
    pub fn live(&self) -> Option<Live> {
        self.imp().live.borrow().clone()
    }

    /// The query as the search parser reads it.
    pub fn parsed(&self) -> ParsedQuery {
        self.imp().parsed.borrow().clone()
    }

    /// The operators drawn as chips in the field.
    pub fn chips(&self) -> Vec<Chip> {
        chips(&self.imp().parsed.borrow())
    }

    /// The commands listed, best first.
    pub fn commands(&self) -> Vec<ActionId> {
        self.imp().commands.borrow().clone()
    }

    /// The folders listed, best first.
    pub fn folders(&self) -> Vec<MailboxId> {
        self.imp().folders.borrow().clone()
    }

    /// The correspondents listed, best first.
    pub fn matched_contacts(&self) -> Vec<ContactHit> {
        self.imp().matched.borrow().clone()
    }

    /// Open the box in `mode`, with nothing typed, and put the keyboard in it.
    pub fn open(&self, mode: Mode) {
        self.begin(mode);
        if let Some(field) = self.imp().field.borrow().as_ref() {
            field.text.grab_focus();
        }
    }

    /// Open the box without touching the focus.
    ///
    /// For when the focus is already arriving under its own steam — a click
    /// in the field, or a Tab into it.
    fn begin(&self, mode: Mode) {
        if self.is_open() && self.mode() == mode {
            return;
        }
        self.imp().open.set(true);
        self.set_query(Query::in_mode(mode));
    }

    /// Close the box and empty it.
    pub fn close(&self) {
        let imp = self.imp();
        imp.open.set(false);
        self.set_query(Query::new());
    }

    /// Put `query` in the box, as though it had been typed.
    pub fn set_query(&self, query: Query) {
        let imp = self.imp();
        *imp.query.borrow_mut() = query.clone();
        if let Some(field) = imp.field.borrow().as_ref() {
            imp.echoing.set(true);
            field.text.set_text(&query.text);
            field.text.set_position(-1);
            imp.echoing.set(false);
        }
        self.refresh();
    }

    /// Move the highlight through the results.
    pub fn move_selection(&self, delta: i32) {
        let imp = self.imp();
        let count = self.row_count() as i32;
        if count == 0 {
            return;
        }
        let current = imp
            .list
            .selected_row()
            .map(|row| row.index())
            .unwrap_or(if delta >= 0 { -1 } else { 0 });
        let next = (current + delta).clamp(0, count - 1);
        if let Some(row) = imp.list.row_at_index(next) {
            imp.list.select_row(Some(&row));
            row.grab_focus();
            // Focus went to the row; the box still has to be typeable.
            if let Some(field) = imp.field.borrow().as_ref() {
                field.text.grab_focus();
            }
        }
    }

    /// Run whatever the box is pointing at.
    pub fn activate(&self) {
        let imp = self.imp();
        match self.mode() {
            Mode::Search => {
                let parsed = imp.parsed.borrow().clone();
                for handler in imp.on_search.borrow().iter() {
                    handler(&parsed);
                }
            }
            Mode::Command => {
                let Some(id) = self
                    .selected_index()
                    .and_then(|index| imp.commands.borrow().get(index).copied())
                else {
                    return;
                };
                for handler in imp.on_command.borrow().iter() {
                    handler(id);
                }
            }
            Mode::Mailbox => {
                let Some(id) = self
                    .selected_index()
                    .and_then(|index| imp.folders.borrow().get(index).copied())
                else {
                    return;
                };
                for handler in imp.on_folder.borrow().iter() {
                    handler(id);
                }
            }
            Mode::Contact => {
                let Some(hit) = self
                    .selected_index()
                    .and_then(|index| imp.matched.borrow().get(index).cloned())
                else {
                    return;
                };
                // The box stays the box. Picking a correspondent writes
                // `from:<address>` into it and drops back into search, so what
                // happens next is an ordinary query the user can build on —
                // and the chip is poppable like any other. Emitted as well,
                // for anything that wants to know who was picked.
                self.set_query(Query {
                    mode: Mode::Search,
                    text: contact_query(&hit),
                });
                for handler in imp.on_contact.borrow().iter() {
                    handler(&hit);
                }
            }
        }
    }

    /// Applies the Backspace rule, and says whether it did something.
    ///
    /// Public so the behaviour can be driven in a test without synthesizing a
    /// key event, which GTK4 gives no supported way to do.
    pub fn press_backspace(&self) -> bool {
        // Backing out of a mode comes first: at the very start of the text
        // there is no character to delete and no chip to pop, so this is the
        // only thing Backspace could sensibly mean.
        if self.caret() == 0
            && let Some(backed_out) = self.query().backspace_at_start()
        {
            self.set_query(backed_out);
            return true;
        }
        if self.mode() != Mode::Search {
            return false;
        }
        match backspace(&self.imp().parsed.borrow(), self.caret()) {
            Backspace::Ordinary => false,
            Backspace::PopChip { query, caret, .. } => {
                let imp = self.imp();
                imp.query.borrow_mut().text = query.clone();
                if let Some(field) = imp.field.borrow().as_ref() {
                    imp.echoing.set(true);
                    field.text.set_text(&query);
                    let chars = query[..caret.min(query.len())].chars().count();
                    field.text.set_position(chars as i32);
                    imp.echoing.set(false);
                }
                self.refresh();
                true
            }
        }
    }

    /// Called when a command is chosen.
    pub fn connect_command(&self, handler: impl Fn(ActionId) + 'static) {
        self.imp().on_command.borrow_mut().push(Box::new(handler));
    }

    /// Called when a folder is chosen.
    pub fn connect_folder(&self, handler: impl Fn(MailboxId) + 'static) {
        self.imp().on_folder.borrow_mut().push(Box::new(handler));
    }

    /// Called when a correspondent is chosen.
    ///
    /// The box has already put `from:<address>` in itself by the time this
    /// runs — this is for anything that wants to know *who*, not for deciding
    /// what picking one means.
    pub fn connect_contact(&self, handler: impl Fn(&ContactHit) + 'static) {
        self.imp().on_contact.borrow_mut().push(Box::new(handler));
    }

    /// Called when a search is run.
    pub fn connect_search(&self, handler: impl Fn(&ParsedQuery) + 'static) {
        self.imp().on_search.borrow_mut().push(Box::new(handler));
    }

    /// Called on every keystroke in search mode.
    pub fn connect_changed(&self, handler: impl Fn(&ParsedQuery) + 'static) {
        self.imp().on_changed.borrow_mut().push(Box::new(handler));
    }

    /// Called when the box is dismissed.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.imp().on_dismissed.borrow_mut().push(Box::new(handler));
    }

    /// Called when `Tab` is pressed while searching. Answer `true` to take
    /// the keyboard; `false` leaves `Tab` meaning what it usually means.
    pub fn connect_tab(&self, handler: impl Fn() -> bool + 'static) {
        self.imp().on_tab.borrow_mut().push(Box::new(handler));
    }

    /// Applies the `Tab` rule, and says whether anything took the keyboard.
    ///
    /// Canvas 2b's footer promises `Tab refine`, so `Tab` out of the query
    /// box goes to the refine chips beside it. Only while searching, and only
    /// when there is something to move to — otherwise `Tab` is still `Tab`,
    /// and a key that silently does nothing is worse than one that does the
    /// ordinary thing.
    ///
    /// Public so the behaviour can be driven in a test without synthesizing a
    /// key event, which GTK4 gives no supported way to do.
    pub fn press_tab(&self) -> bool {
        if self.mode() != Mode::Search {
            return false;
        }
        let handlers = self.imp().on_tab.borrow();
        handlers.iter().any(|handler| handler())
    }

    /// What `Escape` does: dismiss the box.
    ///
    /// Public for the same reason [`press_tab`](Self::press_tab) is — GTK4
    /// gives no supported way to synthesize a key event, and `Escape` is now
    /// load-bearing rather than cosmetic: it is what puts the message list
    /// back on the folder after a search, so it needs to be drivable in a
    /// test that asserts the list came back.
    ///
    /// Distinct from [`close`](Self::close), which empties the box without
    /// telling anyone. Dismissing is the *gesture*; closing is the state.
    pub fn press_escape(&self) {
        self.dismiss();
    }

    // -- internals ----------------------------------------------------------

    fn dismiss(&self) {
        for handler in self.imp().on_dismissed.borrow().iter() {
            handler();
        }
    }

    /// The caret, as a byte offset into the text.
    fn caret(&self) -> usize {
        let imp = self.imp();
        let Some(field) = imp.field.borrow().clone() else {
            return 0;
        };
        let text = field.text.text().to_string();
        let position = field.text.position().max(0) as usize;
        text.char_indices()
            .nth(position)
            .map(|(offset, _)| offset)
            .unwrap_or(text.len())
    }

    /// The user typed; work out what the box is now.
    fn retype(&self, text: &str) {
        let next = self.query().typed(text);
        let absorbed = next.text != text;
        *self.imp().query.borrow_mut() = next.clone();
        if absorbed {
            // The prefix left the text, so the entry has to be told.
            self.set_query(next);
            return;
        }
        self.refresh();
    }

    fn row_count(&self) -> usize {
        let imp = self.imp();
        match self.mode() {
            Mode::Search => 0,
            Mode::Command => imp.commands.borrow().len(),
            Mode::Mailbox => imp.folders.borrow().len(),
            Mode::Contact => imp.matched.borrow().len(),
        }
    }

    fn selected_index(&self) -> Option<usize> {
        self.imp()
            .list
            .selected_row()
            .map(|row| row.index() as usize)
            .filter(|index| *index < self.row_count())
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-finder");
        self.set_halign(gtk::Align::Start);
        self.set_valign(gtk::Align::Start);
        self.set_visible(false);

        imp.list.set_selection_mode(gtk::SelectionMode::Single);
        imp.list.add_css_class("postio-finder-list");
        imp.list.set_accessible_role(gtk::AccessibleRole::ListBox);

        imp.empty.add_css_class("postio-finder-empty");
        imp.empty.set_visible(false);

        imp.scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.scroller.set_propagate_natural_height(true);
        imp.scroller.set_max_content_height(360);
        imp.scroller.set_focusable(false);
        imp.scroller.set_child(Some(&imp.list));

        imp.chips.add_css_class("postio-chips");
        imp.chips.set_visible(false);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&imp.chips);
        column.append(&imp.scroller);
        column.append(&imp.empty);
        self.set_child(Some(&column));

        imp.list.connect_row_activated(glib::clone!(
            #[weak(rename_to = finder)]
            self,
            move |_, _| finder.activate()
        ));

        self.refresh();
    }

    /// Rebuild the rows and redraw the field, every keystroke.
    ///
    /// The registry is a few dozen rows and a query is one line of text, so
    /// there is nothing to gain from an incremental redraw and a stale row to
    /// lose by it.
    fn refresh(&self) {
        let imp = self.imp();
        let query = self.query();

        imp.commands.borrow_mut().clear();
        imp.folders.borrow_mut().clear();
        imp.matched.borrow_mut().clear();
        while let Some(row) = imp.list.first_child() {
            imp.list.remove(&row);
        }

        match query.mode {
            Mode::Search => {
                let parsed = postio_search::parse(&query.text, today());
                *imp.parsed.borrow_mut() = parsed;
                let parsed = imp.parsed.borrow().clone();
                // The readout paces itself: a keystroke reschedules the run
                // rather than starting one, so typing never waits for a
                // search. See `crate::search::Live`.
                if let Some(live) = imp.live.borrow().as_ref() {
                    live.typed(&parsed);
                }
                for handler in imp.on_changed.borrow().iter() {
                    handler(&parsed);
                }
            }
            Mode::Command => {
                // Not searching mail any more, so there is no count to keep
                // showing and nothing in flight worth waiting for.
                if let Some(live) = imp.live.borrow().as_ref() {
                    live.clear();
                }
                let found = entries(
                    &imp.keymap.borrow(),
                    *imp.context.borrow(),
                    *imp.scope.borrow(),
                    &query.text,
                );
                for entry in &found {
                    imp.list.append(&command_row(entry));
                }
                *imp.commands.borrow_mut() = found.iter().map(|entry| entry.id).collect();
            }
            Mode::Mailbox => {
                if let Some(live) = imp.live.borrow().as_ref() {
                    live.clear();
                }
                let found = folders(&imp.mailboxes.borrow(), &query.text);
                for hit in &found {
                    imp.list.append(&folder_row(hit));
                }
                *imp.folders.borrow_mut() = found.iter().map(|hit| hit.id).collect();
            }
            Mode::Contact => {
                if let Some(live) = imp.live.borrow().as_ref() {
                    live.clear();
                }
                let found = contacts(&imp.contacts.borrow(), &query.text);
                for hit in &found {
                    imp.list.append(&contact_row(hit));
                }
                *imp.matched.borrow_mut() = found;
            }
        }

        if let Some(row) = imp.list.row_at_index(0) {
            imp.list.select_row(Some(&row));
        }
        self.render();
    }

    /// Put the field and the plate in step with the mode.
    fn render(&self) {
        let imp = self.imp();
        let query = self.query();
        let open = imp.open.get();

        if let Some(field) = imp.field.borrow().as_ref() {
            field.frame.set_css_classes(if open {
                &["postio-search", "open"]
            } else {
                &["postio-search"]
            });
            let searching = query.mode == Mode::Search;
            field.icon.set_visible(searching);
            field.marker.set_visible(!searching);
            field.marker.set_text(query.mode.marker());
            field
                .text
                .set_placeholder_text(Some(query.mode.placeholder()));
            // The `/` cap invites you in; once you are in, it is noise.
            field.hint.set_visible(!open);
        }

        // The operators, read back under the field rather than drawn inside
        // it. Canvas 2b puts them in the field, and one editable `GtkText`
        // cannot both hold the query and hide the part it has turned into
        // chips — so the query stays whole and legible in the field, and the
        // chips sit under it saying how the parser read it. `crate::search`
        // makes the same call for the same reason: the entry is the truth.
        let drawn = if query.mode == Mode::Search {
            self.chips()
        } else {
            Vec::new()
        };
        while let Some(child) = imp.chips.first_child() {
            imp.chips.remove(&child);
        }
        for chip in &drawn {
            imp.chips.append(&chip_widget(chip));
        }
        imp.chips.set_visible(!drawn.is_empty());

        let count = self.row_count();
        let listing = open && query.mode.has_results();
        // The plate is up when it has something to say: rows to pick from,
        // a reading of the query, or the fact that nothing matched.
        self.set_visible(open && (listing || !drawn.is_empty()));
        imp.scroller.set_visible(listing && count > 0);
        imp.empty.set_visible(listing && count == 0);
        if count == 0 {
            // Never a shrug: say what was looked in, so the next keystroke
            // is an informed one.
            imp.empty.set_text(&match query.mode {
                Mode::Command => format!("No command matches “{}”", query.text),
                Mode::Mailbox => format!("No folder matches “{}”", query.text),
                // Two different empties. Postio has no address book: contacts
                // accumulate from the addresses that have come through the
                // mailbox, so a box with none is a mailbox that has not synced
                // rather than a query that missed — and saying which is the
                // difference between a way forward and a shrug.
                Mode::Contact if imp.contacts.borrow().is_empty() => {
                    "No correspondents yet — Postio learns them from the mail it syncs.".to_string()
                }
                Mode::Contact => format!("No correspondent matches “{}”", query.text),
                Mode::Search => String::new(),
            });
        }
    }
}

/// One command row: the title, and the key that runs it.
fn command_row(entry: &Entry) -> gtk::ListBoxRow {
    let row = row_shell(
        &highlight(entry.title, &entry.positions),
        entry.binding.as_deref(),
    );
    row.update_property(&[gtk::accessible::Property::Label(&match &entry.binding {
        Some(binding) => format!("{}, {binding}", entry.title),
        None => entry.title.to_string(),
    })]);
    row
}

/// One folder row: the folder, and how much of it is unread.
fn folder_row(hit: &FolderHit) -> gtk::ListBoxRow {
    let count = (hit.unread > 0).then(|| hit.unread.to_string());
    let row = row_shell(&highlight(&hit.name, &hit.positions), count.as_deref());
    row.update_property(&[gtk::accessible::Property::Label(&match hit.unread {
        0 => hit.name.clone(),
        unread => format!("{}, {unread} unread", hit.name),
    })]);
    row
}

/// One correspondent: what to call them, and how often they have written.
///
/// The address is the accessible name rather than a second line, because two
/// people called `Ada` are told apart by their address and a screen reader
/// user has no other way to see it. The count is the cap, the same slot the
/// folder rows use for unread.
fn contact_row(hit: &ContactHit) -> gtk::ListBoxRow {
    let seen = (hit.times_seen > 0).then(|| hit.times_seen.to_string());
    let row = row_shell(&highlight(&hit.name, &hit.positions), seen.as_deref());
    row.update_property(&[gtk::accessible::Property::Label(&match hit.times_seen {
        0 => format!("{}, {}", hit.name, hit.address),
        1 => format!("{}, {}, 1 message", hit.name, hit.address),
        seen => format!("{}, {}, {seen} messages", hit.name, hit.address),
    })]);
    row.set_tooltip_text(Some(&hit.address));
    row
}

/// The shared row: a title on the left, a mono cap on the right.
///
/// One arrangement for both modes, and the same one the focused message row
/// uses for its key hints — a key learned here looks the same where it is
/// used. `title` is Pango markup, so the characters the query matched come
/// through in bold.
fn row_shell(title: &str, trailing: Option<&str>) -> gtk::ListBoxRow {
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(None);
    label.set_markup(title);
    label.add_css_class("postio-finder-title");
    label.set_halign(gtk::Align::Start);
    label.set_hexpand(true);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_accessible_role(gtk::AccessibleRole::Presentation);
    line.append(&label);

    if let Some(trailing) = trailing {
        let cap = gtk::Label::new(Some(trailing));
        cap.add_css_class("postio-finder-cap");
        cap.set_accessible_role(gtk::AccessibleRole::Presentation);
        line.append(&cap);
    }

    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-finder-row");
    row.set_child(Some(&line));
    row
}

fn chip_widget(chip: &Chip) -> gtk::Label {
    let label = gtk::Label::new(Some(&chip.label));
    label.add_css_class("postio-chip");
    if chip.negated {
        label.add_css_class("negated");
    }
    if !chip.complete {
        label.add_css_class("partial");
    }
    // Read as what it does, not as the shorthand it is written in.
    label.update_property(&[gtk::accessible::Property::Label(&crate::search::spoken(
        chip,
    ))]);
    label
}

/// The day relative dates resolve against.
///
/// The *local* day, not UTC: `after:yesterday` means the user's yesterday.
/// Read per keystroke rather than cached, so a session left open across
/// midnight does not go on searching against the wrong day.
fn today() -> chrono::NaiveDate {
    chrono::Local::now().date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::ids::AccountId;
    use postio_model::mailbox::{MailboxCounts, MailboxRole};

    #[test]
    fn a_prefix_typed_into_an_empty_box_becomes_the_mode() {
        let box_ = Query::new();
        assert_eq!(box_.mode, Mode::Search);

        let commanding = box_.typed(">");
        assert_eq!(commanding.mode, Mode::Command);
        assert_eq!(commanding.text, "", "the prefix left the text it came from");

        let folder = box_.typed("#lk");
        assert_eq!(folder.mode, Mode::Mailbox);
        assert_eq!(folder.text, "lk");
    }

    #[test]
    fn a_prefix_typed_into_a_mode_is_just_a_character() {
        // Otherwise `>` could never be searched for, and typing it twice
        // would land somewhere nobody asked to be.
        let commanding = Query::in_mode(Mode::Command);
        let deeper = commanding.typed(">");
        assert_eq!(deeper.mode, Mode::Command);
        assert_eq!(deeper.text, ">");
    }

    #[test]
    fn a_prefix_part_way_through_a_query_is_just_a_character() {
        let searching = Query::new().typed("re");
        let more = searching.typed("re>");
        assert_eq!(more.mode, Mode::Search);
        assert_eq!(more.text, "re>");
    }

    #[test]
    fn backspace_at_the_start_gives_up_the_mode_and_keeps_the_words() {
        let commanding = Query::in_mode(Mode::Command).typed("arch");
        let backed_out = commanding
            .backspace_at_start()
            .expect("a mode is something to back out of");
        assert_eq!(backed_out.mode, Mode::Search);
        assert_eq!(
            backed_out.text, "arch",
            "backing out of a mode should not cost what was typed"
        );

        assert_eq!(
            backed_out.backspace_at_start(),
            None,
            "there is nothing behind search, so the entry keeps the keystroke"
        );
    }

    #[test]
    fn every_mode_says_which_one_it_is_and_where_the_keyboard_is() {
        for mode in Mode::ALL {
            assert!(!mode.marker().is_empty());
            assert!(!mode.placeholder().is_empty());
        }
        assert_eq!(Mode::Search.context(), Context::Search);
        assert_eq!(Mode::Command.context(), Context::Palette);
        assert!(
            !Mode::Search.has_results(),
            "search answers in the message list"
        );
        assert!(Mode::Command.has_results());
    }

    fn folder(id: i64, path: &str, role: MailboxRole, unread: u32) -> Mailbox {
        let mut mailbox = Mailbox::new(AccountId::new(1), path, Some('/'));
        mailbox.id = MailboxId::new(id);
        mailbox.role = role;
        mailbox.counts = MailboxCounts {
            total: 100,
            unread,
            flagged: 0,
        };
        mailbox
    }

    #[test]
    fn folders_are_found_the_way_commands_are() {
        let mailboxes = [
            folder(1, "INBOX", MailboxRole::Inbox, 12),
            folder(2, "Archive", MailboxRole::Archive, 0),
            folder(3, "wayland-devel", MailboxRole::Regular, 37),
        ];

        // Subsequence, not substring: the same matcher the command mode uses.
        let hits = folders(&mailboxes, "wd");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "wayland-devel");
        assert_eq!(hits[0].unread, 37);

        // The sidebar's word for a folder, not the server's path.
        let inbox = folders(&mailboxes, "inb");
        assert_eq!(inbox[0].name, "Inbox");

        assert_eq!(
            folders(&mailboxes, "").len(),
            3,
            "an empty query offers every folder"
        );
        assert!(folders(&mailboxes, "zzz").is_empty());
    }

    // -- contacts ---------------------------------------------------------

    fn correspondent(name: Option<&str>, address: &str, times_seen: u32) -> Contact {
        let mut contact = Contact::new(postio_model::EmailAddress::new(name, address));
        contact.times_seen = times_seen;
        contact
    }

    #[test]
    fn a_correspondent_is_found_the_way_a_folder_is() {
        let people = [
            correspondent(Some("Grace Hopper"), "grace@example.com", 40),
            correspondent(Some("Ada Lovelace"), "ada@example.org", 12),
        ];

        // Subsequence, not substring: the same matcher every other mode uses.
        let hits = contacts(&people, "gh");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Grace Hopper");
        assert_eq!(hits[0].address, "grace@example.com");
        assert_eq!(hits[0].times_seen, 40);
    }

    #[test]
    fn a_correspondent_is_findable_by_address_when_the_name_does_not_match() {
        let people = [correspondent(Some("Grace Hopper"), "grace@example.org", 40)];

        assert_eq!(contacts(&people, "example.org").len(), 1);
        assert!(contacts(&people, "example.net").is_empty());
    }

    #[test]
    fn a_correspondent_with_no_name_is_still_something_to_pick() {
        let people = [correspondent(None, "buildbot@example.net", 900)];

        let hits = contacts(&people, "build");
        assert_eq!(hits.len(), 1);
        assert_eq!(
            hits[0].name, "buildbot@example.net",
            "a row with nothing to say is a row nobody can pick"
        );
    }

    #[test]
    fn a_name_the_user_set_beats_the_one_the_headers_carried() {
        let mut contact = correspondent(Some("ADA N."), "ada@example.org", 3);
        contact.name = Some("Ada Lovelace".to_owned());

        assert_eq!(contacts(&[contact], "ada")[0].name, "Ada Lovelace");
    }

    #[test]
    fn an_empty_query_offers_the_people_you_actually_write_to() {
        let people = [
            correspondent(Some("Once Only"), "once@example.com", 1),
            correspondent(Some("Every Day"), "daily@example.com", 900),
        ];

        let hits = contacts(&people, "");
        assert_eq!(hits.len(), 2);
        assert_eq!(
            hits[0].name, "Every Day",
            "with nothing typed, familiarity is the only ranking there is"
        );
    }

    #[test]
    fn picking_a_correspondent_writes_a_query_the_parser_reads_back() {
        let hits = contacts(
            &[correspondent(Some("Grace Hopper"), "grace@example.com", 40)],
            "grace",
        );
        let query = contact_query(&hits[0]);
        assert_eq!(query, "from:grace@example.com");

        // It has to survive being typed back in as an ordinary chip.
        let parsed = postio_search::parse(
            &query,
            chrono::NaiveDate::from_ymd_opt(2026, 8, 23).expect("a real date"),
        );
        let filters: Vec<_> = parsed
            .filters()
            .map(|clause| clause.filter.clone())
            .collect();
        assert_eq!(
            filters,
            vec![postio_search::query::Filter::From(
                "grace@example.com".to_owned()
            )]
        );
    }

    #[test]
    fn an_address_that_could_not_be_typed_back_is_quoted() {
        let hits = contacts(
            &[correspondent(Some("Odd One"), "odd one@example.com", 1)],
            "odd",
        );
        assert_eq!(contact_query(&hits[0]), "from:\"odd one@example.com\"");
    }

    #[test]
    fn at_is_a_mode_like_the_others() {
        let box_ = Query::new();
        let finding = box_.typed("@gra");

        assert_eq!(finding.mode, Mode::Contact);
        assert_eq!(finding.text, "gra");
        assert_eq!(Mode::Contact.marker(), "@");
        assert!(Mode::Contact.has_results());
        assert_eq!(Mode::Contact.context(), Context::Palette);

        // And backing out of it keeps what was typed, like every other mode.
        let backed_out = finding
            .backspace_at_start()
            .expect("a mode is something to back out of");
        assert_eq!(backed_out.mode, Mode::Search);
        assert_eq!(backed_out.text, "gra");
    }

    #[test]
    fn every_mode_has_a_prefix_of_its_own() {
        let mut prefixes: Vec<char> = Mode::ALL.iter().filter_map(|mode| mode.prefix()).collect();
        let count = prefixes.len();
        prefixes.sort_unstable();
        prefixes.dedup();
        assert_eq!(
            prefixes.len(),
            count,
            "two modes on one prefix would make the box ambiguous"
        );
        assert_eq!(count, Mode::ALL.len() - 1, "only Search has no prefix");
    }

    #[test]
    fn a_folder_that_cannot_hold_mail_is_not_somewhere_to_go() {
        let mut noselect = folder(4, "Lists", MailboxRole::Regular, 0);
        noselect.selectable = false;
        assert!(folders(&[noselect], "lists").is_empty());
    }
}
