//! The application window: an `AdwApplicationWindow` wearing the generated
//! design tokens.
//!
//! The canvas' PLATE direction keeps *real* Adwaita window chrome — a genuine
//! `AdwHeaderBar`, the compositor's own controls — so Postio reads as a GNOME
//! application rather than as a canvas drawn inside a bare frame. The Industry
//! identity lives in the type, the steel accent and the hairlines inside that
//! chrome, not in a replacement for it.
//!
//! The window owns four things the panes below it should not have to: the
//! header bar, the breakpoints that decide how many panes fit, the state that
//! has to survive a restart, and the keyboard.
//!
//! # The keyboard
//!
//! Key presses arrive here first, at the capture phase, and go to
//! [`keymap::Resolver`] rather than to a `GtkShortcutController` — see that
//! module for why sequences, per-context `Esc` and "typing always wins" cannot
//! be expressed as accelerators. What comes back is a [`CommandId`], which the
//! window hands to whoever registered with
//! [`connect_command`](Window::connect_command); the window itself only acts on
//! the two commands that are *about* the window, opening the palette and
//! closing what is open.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use postio_core::{ActionId, CommandId, Context};

use crate::cheatsheet::CheatSheet;
use crate::feed::{Feed, Feeds, Folders, MailboxSource, MessageSource};
use crate::finder::{Finder, Mode, Query};
use crate::keymap::{self, ChordFromGdk, KeyContext, Outcome, Resolver};
use crate::list_state::ListStateView;
use crate::list_view::MessageListView;
use crate::settings::SettingsPanel;
use crate::shell::Shell;
use crate::sidebar::Sidebar;
use crate::state::WindowState;
use crate::{header, style};

/// What to call when a key press resolves to a command.
type CommandHandler = Box<dyn Fn(CommandId)>;

/// Switch to a mailbox, the way picking it in the sidebar does. See
/// [`Window::open_mailbox`].
type OpenMailbox = std::rc::Rc<dyn Fn(postio_model::ids::MailboxId)>;

/// What to call with a whole invocation — the verb *and* what it is aimed at.
type ActionHandler = Box<dyn Fn(postio_core::Command)>;

/// What a subscriber to a *registered* command is handed: its id, and nothing
/// else. See [`Window::connect_ext_command`].
type ExtCommandHandler = Box<dyn Fn(postio_core::ExtId)>;
/// See [`Window::connect_keymap`].
type KeymapHandler = Box<dyn Fn(&postio_core::Keymap)>;
/// See [`Window::connect_storage_changed`].
type StorageHandler = Box<dyn Fn(Option<u64>)>;

/// The default size, from canvas 1b: a 1120px board over a 52px header bar.
///
/// Wide enough that the three-pane layout is what a first run actually looks
/// like — a mail client that opens into two panes has already lost the
/// argument about what it is.
pub const DEFAULT_SIZE: (i32, i32) = (1120, 700);

/// How much of a conversation one read asks for.
///
/// One request rather than a paged feed: a thread is a conversation, and the
/// pane already holds every message it is given in memory to stack them.
/// `postio-bench`'s `conversation_rows.rs` measures binding a read-ahead
/// window's worth against a 200-message thread, which is the size this is
/// chosen to clear comfortably; a conversation past it is pathological
/// rather than long.
const THREAD_PAGE: u32 = 500;

/// What the reading pane is showing, as far as the read-clocks care.
///
/// Deliberately coarser than "which widget is visible": the only thing this
/// distinguishes is whose dwell may go on running, and every value that stops
/// both of them is the same value. A surface added later picks one of these
/// rather than choosing which timers to cancel, which is the whole point of
/// #945 -- five call sites each cancelling one of two timers is a shape where
/// a new one silently reproduces #797 somewhere new.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Showing {
    /// One message, which is also the row the list cursor is resting on.
    Message,
    /// A conversation, whose per-message clock is the one that counts.
    Conversation,
    /// The composer has the pane, the pane was emptied, or the user has gone
    /// somewhere else entirely. Nothing anyone is reading, so nothing is read.
    NothingInFrontOfAnybody,
}

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Default)]
    pub struct Window {
        pub shell: OnceCell<Shell>,
        pub sidebar: OnceCell<Sidebar>,
        pub list_state: OnceCell<ListStateView>,
        pub list: OnceCell<MessageListView>,
        /// The list and its named states, together.
        pub list_pane: OnceCell<gtk::Overlay>,
        /// Where opening a conversation reads the whole thread from.
        ///
        /// The message list's own feed owns this too; the window keeps a
        /// handle because a conversation is not a page of the list and cannot
        /// be asked for through it. `None` until `install_feeds`, which is
        /// the state a window built for a test of one widget is in — the pane
        /// then shows what the list model holds, which for a thread row is
        /// one message.
        pub messages: std::cell::RefCell<Option<std::rc::Rc<dyn MessageSource>>>,
        /// Switch to a mailbox the way picking it in the sidebar does: set
        /// by [`install_feeds`](super::Window::install_feeds), so
        /// [`open_mailbox`](super::Window::open_mailbox) is a no-op before
        /// the window has been fed anything to switch to.
        pub open_mailbox: std::cell::RefCell<Option<OpenMailbox>>,
        pub finder: OnceCell<Finder>,
        pub cheatsheet: OnceCell<CheatSheet>,
        /// ADR 0012's first-run keyboard orientation. This crate builds and
        /// places it; `postio-app` decides whether it has anything to say,
        /// because that answer lives in the store.
        pub orientation: OnceCell<crate::orientation::OrientationStrip>,
        /// Installed lazily, on first [`Window::composer`] — nothing before
        /// that call needs it, and the composition root is the one place
        /// that both installs and wires it.
        pub composer: OnceCell<crate::composer::Composer>,
        /// The hardened reader, built into the reading pane on first use.
        ///
        /// Lazy for the reason the composer is: a `WebKitWebView` is the most
        /// expensive widget in the window, and a session that never opens a
        /// message should never pay for one.
        pub reader: OnceCell<crate::reader::Reader>,
        /// The conversation pane, built into the reading pane on first use.
        ///
        /// Beside the reader rather than replacing it: a cursor preview shows
        /// one message and an *opened* conversation shows all of them (ADR
        /// 0015 Q4), so the pane holds whichever the moment calls for.
        pub conversation: OnceCell<crate::conversation::ConversationView>,
        /// Where the reader resolves `cid:` parts from.
        ///
        /// A slot rather than a constructor argument, so the reader can be
        /// built before storage has been wired and start resolving parts the
        /// moment something supplies a source — the same shape the search
        /// preview uses, and for the same reason.
        /// Shared as an `Rc` so the reader's blob-source closure can hold
        /// *this cell* rather than the whole `Window` (#794). The closure
        /// becomes the `Rc<dyn BlobSource>` the reader hands to its
        /// `WebContext`, so capturing the window there closed a cycle
        /// through WebKit; capturing a weak window instead made inline
        /// images silently fail to decode once the last strong reference
        /// went. It needs the cell, not the window, and it needs it live —
        /// `set_blob_source` runs after the reader is built.
        pub blobs:
            std::rc::Rc<std::cell::RefCell<Option<std::rc::Rc<dyn crate::reader::BlobSource>>>>,
        /// Where the reader's remote-image allow list is read from and saved
        /// back to, when it should not be the real one.
        ///
        /// `None` — the shipping case — means
        /// `$XDG_STATE_HOME/postio/remote-images.ini`. See
        /// [`Window::set_allowlist_path`](super::Window::set_allowlist_path) for
        /// why a test needs to say
        /// otherwise.
        pub allowlist_path: std::cell::RefCell<Option<std::path::PathBuf>>,
        /// Whether the reader has a message to show.
        ///
        /// The reading pane holds both the reader and the composer, so "is
        /// there something to read" and "is the composer up" are different
        /// questions and the pane needs both to decide what to draw.
        pub reading: std::cell::Cell<bool>,
        /// The header's own `Compose` button — kept so the composer can make
        /// it say `Composing` while it has the reading pane. The rest of
        /// `Header` has no other reader today, so only the button is worth
        /// keeping rather than the whole struct.
        pub compose_button: OnceCell<gtk::Button>,
        /// *Archived 12 messages — Undo.* Built alongside the rest of the
        /// window rather than lazily: every window needs somewhere to put
        /// this, the same way every window needs a header.
        pub toast: OnceCell<crate::toast::Toast>,

        pub settings: OnceCell<SettingsPanel>,
        /// What a message is made of. See [`crate::parts`].
        pub parts: OnceCell<crate::parts::PartsPanel>,
        /// The pane that had the keyboard when the box opened.
        pub before_finder: std::cell::Cell<Option<(Context, crate::shell::Pane)>>,
        /// Whether the box that is open was opened to answer a `Move`.
        ///
        /// `m` sends `Command::Move { to: None }`, and `None` means "ask the
        /// user" — so the window opens the folder picker and has to remember
        /// *why*, or the folder that comes back would be navigated to instead
        /// of moved into. Established by
        /// [`open_finder`](super::Window::open_finder), which is the one way
        /// the box is ever opened, so a pick can never answer a move the user
        /// abandoned two openings ago.
        pub pending_move: std::cell::Cell<bool>,
        /// The context that had the keyboard before it went to the folders,
        /// so `Esc` puts it back where it was rather than guessing `List`.
        pub before_sidebar: std::cell::Cell<Option<Context>>,
        /// The context that had the keyboard before it went to the parts
        /// panel, restored when the panel closes — see `before_sidebar`.
        pub before_parts: std::cell::Cell<Option<Context>>,
        /// The context that had the keyboard before it went to the account
        /// list in settings — see `before_sidebar` (#471).
        pub before_accounts: std::cell::Cell<Option<Context>>,
        /// The context that had the keyboard before it went to the
        /// keybinding list in settings — see `before_sidebar` (#1016).
        pub before_keys: std::cell::Cell<Option<Context>>,
        /// Set once `keys_list`'s own `EventControllerFocus` has been
        /// added — never during `Window::new`'s own construction. See
        /// `Window::ensure_keys_focus_controller`'s own doc for why.
        pub keys_focus_installed: std::cell::Cell<bool>,
        pub overlay: OnceCell<gtk::Overlay>,
        pub resolver: OnceCell<std::cell::RefCell<Resolver>>,
        /// `None` until `build` sets it; the accessor reads it as `List`.
        pub context: std::cell::Cell<Option<Context>>,
        /// What the mail on screen belongs to. Beside `context` because the
        /// two together are what decides whether a command is offered (#182).
        pub scope: std::cell::Cell<postio_core::Scope>,
        pub commands: std::cell::RefCell<Vec<CommandHandler>>,
        /// Handlers for whole invocations, which the mouse produces — see
        /// [`Window::connect_action`](super::Window::connect_action).
        pub actions: std::cell::RefCell<Vec<ActionHandler>>,
        /// Handlers for commands registered at runtime — see
        /// [`Window::connect_ext_command`](super::Window::connect_ext_command).
        pub ext_commands: std::cell::RefCell<Vec<ExtCommandHandler>>,
        /// Surfaces outside this window that still owe their key hints to the
        /// live keymap — see
        /// [`Window::connect_keymap`](super::Window::connect_keymap).
        pub keymaps: std::cell::RefCell<Vec<KeymapHandler>>,
        /// Whoever owns the store side of `[storage] max_bytes` — see
        /// [`Window::connect_storage_changed`](super::Window::connect_storage_changed).
        pub storage_changed: std::cell::RefCell<Vec<StorageHandler>>,
        /// The keymap currently in force, once one has been applied, so a
        /// surface built later can be handed it rather than waiting for the
        /// next edit.
        ///
        /// `None` until then rather than `Keymap::default()`, which is empty:
        /// handing that to a surface would clear every hint it had drawn from
        /// the registry defaults, which is worse than the defaults.
        pub keymap: std::cell::RefCell<Option<postio_core::Keymap>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Window {
        const NAME: &'static str = "PostioWindow";
        type Type = super::Window;
        type ParentType = adw::ApplicationWindow;
    }

    impl ObjectImpl for Window {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Window {}
    impl WindowImpl for Window {}
    impl ApplicationWindowImpl for Window {}
    impl AdwApplicationWindowImpl for Window {}
}

glib::wrapper! {
    /// Postio's main window.
    pub struct Window(ObjectSubclass<imp::Window>)
        @extends adw::ApplicationWindow, gtk::ApplicationWindow, gtk::Window, gtk::Widget,
        @implements gio::ActionGroup, gio::ActionMap, gtk::Accessible, gtk::Buildable,
                    gtk::ConstraintTarget, gtk::Native, gtk::Root, gtk::ShortcutManager;
}

impl Window {
    /// A window belonging to `application`.
    pub fn new(application: &impl IsA<gtk::Application>) -> Self {
        glib::Object::builder()
            .property("application", application.as_ref())
            .build()
    }

    /// If nothing has claimed the keyboard yet, claims it for the message
    /// list before presenting -- the widget [`Context::List`] (the default
    /// context, set once in `constructed`) already claims.
    ///
    /// Order matters: GTK only picks its own initial-focus fallback *if
    /// none is set yet* when the window maps, and that map happens
    /// synchronously inside `present()` itself -- so claiming this
    /// afterwards is already too late; whatever the fallback landed on has
    /// had its side effects by the time control returns. Left to that
    /// fallback, the window's very first real focus is whatever its
    /// tab-order search finds, which has no notion of "the list is the
    /// intended default". A still-syncing account has no folders yet, so
    /// with a pinned saved search already on the sidebar (`app.rs` loads
    /// `config.toml` before this runs) that row is the only focusable thing
    /// in the window -- and focusing a row in a `GtkListBox` selects it in
    /// `SelectionMode::Single`, which runs the search exactly as a click
    /// would, before a single key is pressed (#614).
    ///
    /// Gated on nothing already having focus so this does not steal it back
    /// from a caller that claimed something more specific first -- the
    /// composer opening straight into its first field before the window is
    /// ever mapped (`composer::focus_first`) being the one that exists
    /// today. Shadows `GtkWindowExt::present` deliberately, so every call
    /// site -- `app.rs` included -- gets this for free.
    pub fn present(&self) {
        if gtk::prelude::GtkWindowExt::focus(self).is_none() {
            self.list().grab_focus();
        }
        gtk::prelude::GtkWindowExt::present(self);
    }

    /// The three panes, for whoever is filling them.
    pub fn shell(&self) -> Shell {
        self.imp()
            .shell
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// The folder list and the sync status line.
    pub fn sidebar(&self) -> Sidebar {
        self.imp()
            .sidebar
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// The message list: canvas 1b's header, and the rows under it.
    pub fn list(&self) -> MessageListView {
        self.imp().list.get().expect("built in constructed").clone()
    }

    /// What a message is made of, per canvas 3g.
    pub fn parts(&self) -> crate::parts::PartsPanel {
        self.imp()
            .parts
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Show the structure of a message whose own content type is `root`.
    ///
    /// Metadata only — see [`crate::parts`]. Nothing is fetched by opening
    /// this, which is the whole reason it can be opened for a message the
    /// application has never downloaded.
    pub fn open_parts(&self, root: &str, attachments: &[postio_model::Attachment]) {
        let panel = self.parts();
        panel.show_parts(root, attachments);
        panel.set_visible(true);
        panel.focus_tree();
        // A real context, not a focus flag — the same reason the sidebar
        // needed one. Without it `j` here reached the window's own resolver
        // first and moved the message selection instead of walking the
        // tree; see `postio-14b`.
        if self.context() != Context::Parts {
            self.imp().before_parts.set(Some(self.context()));
            self.set_context(Context::Parts);
        }
    }

    /// Put the parts panel away.
    pub fn close_parts(&self) {
        self.parts().set_visible(false);
        if self.context() == Context::Parts {
            let previous = self.imp().before_parts.take().unwrap_or(Context::List);
            self.set_context(previous);
        }
    }

    /// The rows the list is holding for `thread`.
    ///
    /// Read off the model rather than asked for, which is what lets the pane
    /// answer the cursor without waiting for a query. The model is windowed,
    /// so this is what the list has paged in — the header says as much when
    /// that is fewer than the row's own thread count.
    fn thread_rows(&self, thread: postio_model::ids::ThreadId) -> Vec<crate::list::Row> {
        let model = self.list().model();
        let mut rows = Vec::new();
        for index in 0..model.n_items() {
            let Some(row) = model
                .item(index)
                .and_then(|item| item.downcast::<crate::list::MessageRow>().ok())
                .and_then(|item| item.row())
            else {
                continue;
            };
            if row.thread == Some(thread) {
                rows.push(row);
            }
        }
        rows
    }

    /// Show `row`'s whole conversation in the reading pane (ADR 0015 Q4).
    ///
    /// The half of opening a conversation that is about content rather than
    /// the index column: landing on a thread row — the cursor, a click,
    /// `Enter` — opens the conversation here, and `t`'s own job is only ever
    /// the column (#755). Same local-first shape too: what the list already
    /// holds goes up synchronously, and the whole conversation — cross-folder,
    /// pages the list never scrolled to — supersedes it when the
    /// `ListScope::Thread` read answers.
    pub fn open_conversation(&self, row: &crate::list::Row) {
        let Some(id) = row.thread else { return };
        // The list holds one row per conversation, so this first paint is
        // usually one entry. It is still worth doing: the pane answers the
        // cursor inside the interaction budget, and the rest of the
        // conversation arrives under it.
        // Whether a person chose this row. The autoselect routes here too —
        // a window must not open beside an empty pane (#601) — but only a
        // chosen landing may start the read-clock, which is #71's rule on
        // the list applied to the pane it now feeds.
        let chosen = self.list().landed();
        self.show_conversation(crate::conversation::arrange(
            &self.thread_rows(id),
            crate::conversation::Order::Oldest,
            false,
        ));
        if !chosen {
            self.conversation().cancel_dwell();
        }
        // What the *subset* open chose to focus. The read below completes
        // the opening, so its policy runs again over the whole conversation
        // — the real first unread may be a message the list never held —
        // unless focus has already moved, which makes it the user's.
        let opened_on = self.conversation().focused();

        let Some(source) = self.imp().messages.borrow().clone() else {
            return;
        };
        let future = source.fetch(crate::feed::PageRequest {
            scope: crate::feed::ListScope::Thread(id),
            page: 0,
            offset: 0,
            limit: THREAD_PAGE,
        });
        glib::spawn_future_local(glib::clone!(
            #[weak(rename_to = window)]
            self,
            async move {
                // POSTIO-GLIB-SAFE: `MessageSource::fetch` is a trait method,
                // and the trait's contract is that what it returns is pollable
                // on the main context -- `postio-app` implements it by
                // spawning the runtime work and handing back a channel
                // receive.
                match future.await {
                    Ok(page) => window.fill_conversation(id, page.rows, opened_on, chosen),
                    // The pane keeps what the list gave it — a subset rather
                    // than nothing. Worth a line, not a banner.
                    Err(message) => tracing::debug!(message, "the thread could not be read"),
                }
            }
        ));
    }

    /// Put a conversation in the reading pane, and the reader aside.
    ///
    /// The tail of [`Window::show_thread`] that is not about the column, so
    /// [`Window::open_conversation`] can raise the pane without one. Expects
    /// `rows` oldest first — [`crate::conversation::arrange`]'s order — because
    /// Whether the conversation pane is the one on screen.
    ///
    /// Asked of the slot and of the widget, never of `reading`: that flag is
    /// about a *single message* being open, and `show_conversation` leaves it
    /// alone -- so "not reading" is true both when the pane is empty and when
    /// a whole conversation is sitting in it.
    fn conversation_visible(&self) -> bool {
        self.imp()
            .conversation
            .get()
            .is_some_and(|pane| pane.widget().is_visible())
    }

    /// What the reading pane is showing now.
    ///
    /// #945: the read-clocks were two independent timers with five
    /// independent cancel points, and every new place that changes what the
    /// reader is looking at had to remember to stop the right one. #797 is
    /// what forgetting costs -- mail marked read that nobody looked at, which
    /// is unrecoverable in the way that matters: an unread the user never saw
    /// is gone from the count and from the bolding that would have brought
    /// them back to it.
    ///
    /// So the cancelling is one decision in one place, and every site that
    /// changes what is in front of the reader states what it is showing
    /// instead of naming a timer.
    fn reader_now_shows(&self, showing: Showing) {
        // The list's clock measures "this row was in front of a person long
        // enough to have been read" (#71). A single message in the pane *is*
        // that row -- the pane follows the cursor (#70) -- so showing one is
        // the only thing that does not stop it.
        if showing != Showing::Message {
            self.list().cancel_dwell();
        }
        // The conversation's measures the same thing about its own focused
        // message (ADR 0015 Q4), so it stops for everything except the
        // conversation. Asked of the slot rather than of `conversation()`,
        // which would *build* the pane: a window that has never opened one
        // has no clock to stop.
        if showing != Showing::Conversation
            && let Some(pane) = self.imp().conversation.get()
        {
            pane.cancel_dwell();
        }
    }

    /// that is how the pane numbers them.
    pub fn show_conversation(&self, rows: Vec<crate::list::Row>) {
        // The list's dwell measures "this row was in front of a person long
        // enough to have been read" (#71), and a thread row's id is its
        // *representative* -- the newest message in that folder. Once the
        // conversation is what the reader is looking at, the row is not, and
        // letting the clock run marks a message read that focus may never
        // reach: "opened the thread, all six read", which ADR 0015 Q4 forbids
        // in those words. Reading inside a conversation is the pane's own
        // per-message dwell, driven by focus.
        //
        // Here rather than in `show_thread`, because that is one of two ways
        // in: #755 made the list open a conversation directly, without the
        // conversation pane. This is the single point both routes pass through
        // -- the pane becoming visible is exactly the moment the row stops
        // being what is in front of the reader.
        //
        // Deliberate, next to the two cases that already stop this clock for
        // the same reason: the composer taking the pane, and the window going
        // inactive. It used to depend on `sync_reading_pane`'s
        // `if !self.reading()` happening to run first, which is timing rather
        // than a decision -- and on a slow machine it did not. #797.
        //
        // Said as "the pane is showing a conversation now" rather than as
        // "cancel the list's timer", so the next surface that takes the pane
        // inherits the decision instead of having to rediscover it (#945).
        self.reader_now_shows(Showing::Conversation);

        let pane = self.conversation();
        pane.open(rows);
        pane.widget().set_visible(true);
        if let Some(reader) = self.imp().reader.get() {
            reader.widget().set_visible(false);
        }
    }

    /// Whether the conversation pane is up and showing `thread`.
    ///
    /// What lets a caller skip re-opening a conversation the pane already
    /// has — re-running the opening policy would move focus out from under
    /// somebody who had already walked away from the first unread.
    pub fn conversation_on(&self, thread: postio_model::ids::ThreadId) -> bool {
        self.imp().conversation.get().is_some_and(|pane| {
            pane.widget().is_visible()
                && pane
                    .rows()
                    .first()
                    .is_some_and(|row| row.thread == Some(thread))
        })
    }

    /// Put the whole conversation's rows into the pane, if it is still
    /// showing `thread`.
    ///
    /// Focus is the user's if they have moved it since the subset opened —
    /// restored, same contract as [`Window::refill_conversation`] — and the
    /// opening policy's otherwise: this read *completes* the opening, and
    /// the first unread of the whole conversation may be a message the
    /// subset never held.
    fn fill_conversation(
        &self,
        thread: postio_model::ids::ThreadId,
        rows: Vec<crate::list::Row>,
        opened_on: Option<postio_model::ids::MessageId>,
        chosen: bool,
    ) {
        if !self.conversation_on(thread) {
            return;
        }
        let pane = self.conversation();
        let focused = pane.focused();
        pane.open(crate::conversation::arrange(
            &rows,
            crate::conversation::Order::Oldest,
            false,
        ));
        if let Some(focused) = focused
            && opened_on.is_some_and(|opened_on| opened_on != focused)
            && pane.rows().iter().any(|row| row.id == focused)
        {
            // The user moved focus since the subset opened, so this is their
            // gesture continuing — the dwell the restore starts is theirs.
            pane.focus_message(focused);
        } else if !chosen {
            // Still the autoselect's opening: the policy's focus stands, and
            // the read-clock stays off exactly as it did the first time.
            pane.cancel_dwell();
        }
    }

    /// Whether a composer has been built yet.
    ///
    /// For the tests that guard [`composer`](Self::composer)'s laziness: it
    /// is easy to reach for the composer from something that runs on every
    /// window — `apply_keymap` did (#828) — and the cost of that is a WebKit
    /// editor built in every window that never composes, which shows up as a
    /// test suite timing out rather than as anything obviously wrong.
    #[doc(hidden)]
    pub fn has_composer(&self) -> bool {
        self.imp().composer.get().is_some()
    }

    /// The composer, installing it into the reading pane the first time
    /// anyone asks.
    ///
    /// Lazy rather than built alongside the other panes in `constructed`: a
    /// window used only for a test of, say, the sidebar has no reason to pay
    /// for a composer nobody opens. Whoever wires storage to it — the
    /// composition root — is the one place that needs this at all.
    pub fn composer(&self) -> crate::composer::Composer {
        if let Some(composer) = self.imp().composer.get() {
            return composer.clone();
        }
        let composer = crate::composer::install(self);
        // The two share the reading pane, so each hand-over is a swap. Wired
        // once, here, because this is the moment the second of the pair comes
        // into existence — and because neither widget should have to know the
        // other one does.
        // Weak, for the same reason `postio-app`'s composition wiring is
        // (#1072, #794): `composer` is a child this window owns via
        // `imp().composer`, so a strong clone here is a cycle that keeps the
        // window alive for the life of the process. A window that has gone
        // has no reading pane left to sync.
        composer.connect_opened({
            let window = glib::object::ObjectExt::downgrade(self);
            move || {
                if let Some(window) = window.upgrade() {
                    window.sync_reading_pane();
                }
            }
        });
        composer.connect_closed({
            let window = glib::object::ObjectExt::downgrade(self);
            move |_| {
                if let Some(window) = window.upgrade() {
                    window.sync_reading_pane();
                }
            }
        });
        // Built after the keymap was applied, so it starts on the user's
        // bindings rather than the registry defaults `build_actions` drew
        // with. `apply_keymap` deliberately does not reach for a composer
        // that does not exist yet, so this is the other half of that (#828).
        if let Some(keymap) = self.imp().keymap.borrow().as_ref() {
            composer.set_keymap(keymap);
        }
        let _ = self.imp().composer.set(composer.clone());
        composer
    }

    /// The reader, installing it into the reading pane the first time anyone
    /// asks.
    ///
    /// # Why the window mounts this and not the composition root
    ///
    /// The reading pane holds two things that must never be on screen at
    /// once — this and the composer, which takes the pane over. Something has
    /// to own that swap, and it cannot be either of them: a reader that hid
    /// itself when a composer appeared would have to know composers exist.
    /// The window is what holds both.
    /// A hardened reader wired to this window's blob source and allow list.
    ///
    /// The reading pane's own reader is one of these; the conversation pane
    /// asks for one per expanded message (ADR 0015 Q4). Built here rather
    /// than by the caller because the blob source and the allow-list path are
    /// the window's, and a second construction site is how two readers end up
    /// hardened differently.
    ///
    /// **Not cheap.** Each one is a `WebKitWebView`. Whoever calls this is
    /// responsible for calling it as few times as the design allows — see
    /// `conversation::EAGER_EXPANSION_CAP`.
    pub fn new_reader(&self) -> crate::reader::Reader {
        // Read through the slot on every request rather than captured, so a
        // source wired after the reader was built still resolves parts.
        // Weak, and this one is load-bearing (#794). The closure becomes the
        // `Rc<dyn BlobSource>` the reader hands to its `WebContext`, so a
        // strong capture closes
        //
        //     Window -> Reader -> WebContext -> scheme handler -> closure
        //
        // back onto the Window. That cycle lives inside WebKit's context,
        // which is why destroying the window never broke it and why the
        // WebProcess was still attached at `exit()`.
        // Captures the blob cell, not the window: no reference back to the
        // `Window` at all, so there is no cycle to break and nothing to
        // upgrade. See the field's own comment (#794).
        let source = {
            let blobs = std::rc::Rc::clone(&self.imp().blobs);
            move |content_id: &str| {
                let blobs = blobs.borrow();
                blobs.as_ref().and_then(|blobs| blobs.resolve(content_id))
            }
        };
        let source = std::rc::Rc::new(source);
        match self.imp().allowlist_path.borrow().clone() {
            Some(path) => crate::reader::Reader::with_allowlist(
                source,
                crate::reader::RemoteImageAllowList::load_from(&path),
                path,
            ),
            None => crate::reader::Reader::new(source),
        }
    }

    /// Put the column's current rows into the conversation pane, keeping the
    /// current message where it is.
    ///
    /// Called when the rest of a conversation arrives from the store. Focus
    /// is restored rather than recomputed: the opening policy is for
    /// *opening*, and re-running it a moment later would move the reader out
    /// from under somebody who had already pressed `K`.
    pub fn refill_conversation(&self) {
        let Some(pane) = self.imp().conversation.get() else {
            return;
        };
        if pane.is_empty() {
            return;
        }
        let focused = pane.focused();
        let rows = pane.rows();
        pane.open(rows);
        if let Some(focused) = focused
            && pane.rows().iter().any(|row| row.id == focused)
        {
            pane.focus_message(focused);
        }
    }

    /// The conversation pane, built on first use.
    ///
    /// Mounted into the same box as the reader and hidden until a
    /// conversation is opened. Both live there because they answer different
    /// moments: a row that is one message previews that message, and a row
    /// that stands for a conversation fills the pane with all of it.
    pub fn conversation(&self) -> crate::conversation::ConversationView {
        if let Some(pane) = self.imp().conversation.get() {
            return pane.clone();
        }
        let pane = crate::conversation::ConversationView::new();
        let widget = pane.widget();
        widget.set_vexpand(true);
        widget.set_hexpand(true);
        widget.set_visible(false);
        self.shell().reader().append(&widget);

        let _ = self.imp().conversation.set(pane.clone());
        pane
    }

    /// The reader currently drawing a message.
    ///
    /// There is more than one, and which one a per-message verb aims at is
    /// not a fact about the window: the conversation pane builds a reader per
    /// expanded message, so `View original` in a stacked conversation means
    /// *the focused message's* reader, not the single-message one behind it.
    /// Falls back to that one, which is what a folder row that is not a
    /// conversation puts on screen.
    fn reader_showing(&self) -> crate::reader::Reader {
        self.imp()
            .conversation
            .get()
            .and_then(|pane| pane.focused())
            .and_then(|message| self.conversation().reader_for(message))
            .unwrap_or_else(|| self.reader())
    }

    pub fn reader(&self) -> crate::reader::Reader {
        if let Some(reader) = self.imp().reader.get() {
            return reader.clone();
        }
        let reader = self.new_reader();
        let widget = reader.widget();
        widget.set_vexpand(true);
        widget.set_hexpand(true);
        self.shell().reader().append(&widget);
        // The pane's arbiter owns whether this widget shows (#502): hidden
        // now — nothing to read yet — and visible exactly while the reader
        // is the pane's occupant. Nothing else may touch its visibility.
        self.shell()
            .register_reader_occupant(crate::shell::ReaderOccupant::Reader, &widget);

        // The parts panel's held-back count follows every render, not just
        // the first: the banner's "show once" and "always allow" change how
        // much is being held back, and a badge that only updated on the
        // initial render would go stale the moment either is used.
        // Weak, like the two below it: `Reader` stores these handlers, and
        // the window's imp stores the `Reader`, so a strong capture is a
        // cycle. All three had to go weak before the window could be freed
        // -- any one of them alone kept it alive, which is why fixing them
        // one at a time looked like no fix at all (#794).
        reader.connect_rendered({
            let window = self.downgrade();
            move |held| {
                if let Some(window) = window.upgrade() {
                    window
                        .parts()
                        .set_held_back(held.remote_images, held.trackers)
                }
            }
        });

        // The action bar's mouse runs the same commands the keyboard does,
        // through the same path `list_view`'s row actions use: a button that
        // acted directly would be a second implementation of a verb the
        // registry already owns.
        reader.connect_command({
            let window = self.downgrade();
            move |command| {
                if let Some(window) = window.upgrade() {
                    window.act(command)
                }
            }
        });

        let _ = self.imp().reader.set(reader.clone());
        reader
    }

    /// Where the reader resolves `cid:` parts from.
    ///
    /// Set by whoever wires storage. Until it is, an inline image simply does
    /// not resolve — which is the same thing that happens for a part the
    /// store has never fetched, and is deliberately not a network request.
    pub fn set_blob_source(&self, blobs: std::rc::Rc<dyn crate::reader::BlobSource>) {
        *self.imp().blobs.borrow_mut() = Some(blobs);
    }

    /// Read and write the remote-image allow list at `path` instead of the
    /// real `$XDG_STATE_HOME/postio/remote-images.ini`.
    ///
    /// What a test points at a scratch file, and the only caller there is:
    /// the application wants the real one.
    ///
    /// # Why this exists (#215)
    ///
    /// A `Window` builds its own [`crate::reader::Reader`], and that reader
    /// loaded the developer's own standing allow list. So a test asserting
    /// that a remote image *is* held back was really asserting something
    /// about the machine: allow-list the sender it uses — by looking at the
    /// app once, or by running a test that clicked "always allow" — and the
    /// body renders with images permitted, nothing is held back, and the test
    /// fails on every commit and every branch, because the cause is not in
    /// the tree at all. That cost a p1 and a bisect that could not find
    /// anything, since there was nothing to find.
    ///
    /// Unlike [`set_blob_source`], this has to be set **before** anything
    /// asks for the reader: the list loads once, when the reader is built,
    /// and stays in memory for its life (see
    /// [`crate::reader::Reader::new`]) precisely so there is never a second
    /// opinion about who is allow-listed.
    ///
    /// [`set_blob_source`]: Self::set_blob_source
    pub fn set_allowlist_path(&self, path: &std::path::Path) {
        debug_assert!(
            self.imp().reader.get().is_none(),
            "set_allowlist_path must come before the reader is built, or the \
             list it already loaded is the one that stays"
        );
        *self.imp().allowlist_path.borrow_mut() = Some(path.to_path_buf());
    }

    /// Show a message in the reading pane.
    ///
    /// `sender` is the allow-list key: remote images stay blocked until this
    /// sender is allowed, which is [`crate::reader::Reader`]'s own rule and
    /// is not something this can bypass.
    pub fn show_message(&self, body: &postio_model::MessageBody, sender: Option<&str>) {
        let reader = self.reader();
        reader.render(body, sender);
        // A single message takes the pane back from a conversation (#755):
        // the cursor moved to a row that is not one, so the stack would be
        // showing mail the user has left.
        self.take_pane_from_conversation();
        // The conversation the user has left must not go on being read: its
        // clock is running on a message that is no longer in front of them.
        // The list's keeps running -- a single message in the pane *is* the
        // row the cursor is on, and stopping it here would mean nothing in
        // the list ever went read at all (#945).
        self.reader_now_shows(Showing::Message);
        self.imp().reading.set(true);
        // A claim, not a sync: opening a message is the gesture that takes
        // the pane — over the search preview when `Enter` opens a result —
        // where `sync_reading_pane` only keeps the arbiter's flag current.
        self.shell().claim_reading();
        self.sync_reading_pane();
    }

    /// Show the message, but say why its body is not here.
    ///
    /// The pane is *open* on a message either way — this is a message with
    /// no body yet, not the absence of a message — so `reading` is set and
    /// the pane is revealed exactly as [`show_message`] does. The difference
    /// the user sees is the plate instead of the body, which is the whole
    /// point of #70: a mailbox mid-backfill must not look like a broken app.
    ///
    /// [`show_message`]: Self::show_message
    pub fn show_absent(&self, state: crate::reader::Absent) {
        self.reader().show_absent(state);
        // As [`show_message`]: a wait plate is still a single message.
        self.take_pane_from_conversation();
        self.reader_now_shows(Showing::Message);
        self.imp().reading.set(true);
        self.shell().claim_reading();
        self.sync_reading_pane();
    }

    /// Hide the conversation pane so the reader can have its place.
    ///
    /// Only the manual half of [`Window::show_conversation`]'s swap is
    /// undone here: the reader's own widget is the arbiter's to show, and
    /// `claim_reading` follows every call to this — putting it back by hand
    /// would also put it back while the composer holds the pane.
    fn take_pane_from_conversation(&self) {
        if let Some(pane) = self.imp().conversation.get() {
            pane.widget().set_visible(false);
        }
    }

    /// Empty the reading pane — the folder changed, or the message went away.
    pub fn clear_reader(&self) {
        if let Some(reader) = self.imp().reader.get() {
            reader.clear();
        }
        self.imp().reading.set(false);
        // The folder changed or the message went away, so nothing in the
        // pane is in front of anybody (#945).
        self.reader_now_shows(Showing::NothingInFrontOfAnybody);
        self.sync_reading_pane();
    }

    /// Whether the reading pane is showing a message right now.
    pub fn reading(&self) -> bool {
        self.imp().reading.get() && !self.composing()
    }

    /// Whether the composer has the reading pane.
    ///
    /// Asks the slot rather than [`Window::composer`], which would *install* a
    /// composer just to be told there is not one.
    fn composing(&self) -> bool {
        self.imp()
            .composer
            .get()
            .is_some_and(|composer| composer.is_open())
    }

    /// Give the pane to whichever of the two should have it.
    ///
    /// The composer wins while it is open: it took the pane over on purpose,
    /// and a reply drawn on top of the message being replied to is the bug
    /// this exists to prevent.
    fn sync_reading_pane(&self) {
        // Through the pane's one owner (#502), never `set_visible` directly:
        // whether the reader actually shows depends on who else is active —
        // the composer, the search preview — and the arbiter is what knows.
        // The *raw* flag, not `reading()`: that accessor reports false while
        // the composer is up, and syncing its masked answer into the arbiter
        // erased the fact that a message was open — which is exactly the
        // fact the arbiter needs to restore it when the composer leaves.
        if self.imp().reader.get().is_some() {
            self.shell().set_reading(self.imp().reading.get());
        }
        // Which pane the narrowest mode shows. Below `MESSAGE_FOCUSED_WIDTH`
        // there is room for the list *or* the reader, and `focused_pane`
        // decides -- but nothing moved it when a message opened, so opening
        // one filled a reader the shell was not showing and the list stayed
        // on screen. The primary action of a mail client did nothing (#825).
        //
        // Declared unconditionally, never behind a mode check:
        // `set_focused_pane` is documented as harmless in the wider modes --
        // it is recorded and takes effect if the window is ever narrowed --
        // and a navigation handler that asks what mode it is in is one that
        // will be wrong in the mode its author was not thinking about
        // (ADR 0024).
        self.shell().set_focused_pane(if self.imp().reading.get() {
            crate::shell::Pane::Reader
        } else {
            crate::shell::Pane::List
        });
        // The dwell timers (#71) measure "this message was in front of a
        // person for long enough to have been read", so this is the moment to
        // restate what is in front of them.
        //
        // `!self.reading()` used to stand in for "the composer took the
        // pane", and it cannot: it is equally true while a conversation is
        // open, because `show_conversation` does not set that flag. Stopping
        // both clocks on it would mean a conversation could never be read at
        // all, and stopping only the list's -- what it did before -- meant
        // the composer taking the pane left the conversation's running.
        // Naming the surface answers both (#945).
        self.reader_now_shows(if self.composing() {
            Showing::NothingInFrontOfAnybody
        } else if self.conversation_visible() {
            Showing::Conversation
        } else if self.reading() {
            Showing::Message
        } else {
            Showing::NothingInFrontOfAnybody
        });
    }

    /// The header's `Compose` button, once `build` has run. `None` only
    /// before `constructed` finishes, which nothing outside this module ever
    /// observes.
    pub fn compose_button(&self) -> Option<gtk::Button> {
        self.imp().compose_button.get().cloned()
    }

    /// *Archived 12 messages — Undo.* Whoever applies a
    /// [`postio_core::Command`] and gets back an undoable
    /// [`postio_core::Event::ActionCompleted`] calls this with it; `u` and
    /// the toast's own button both end up at `win.undo`, which reaches
    /// [`Window::connect_command`] the same way the keyboard does.
    pub fn show_action_completed(&self, description: &str, undoable: bool) {
        if let Some(toast) = self.imp().toast.get() {
            toast.show_action_completed(description, undoable);
        }
    }

    /// *Archived 12 messages, undone.* What `u` (or the toast's button)
    /// leaves on screen once it has run.
    pub fn show_undo_performed(&self, description: &str) {
        if let Some(toast) = self.imp().toast.get() {
            toast.show_undo_performed(description);
        }
    }

    /// *Account removed — Undo*, with `on_undo` reachable only from this
    /// toast's own button — see [`crate::toast::Toast::show_removable`].
    pub fn show_removable_toast(&self, description: &str, on_undo: impl Fn() + 'static) {
        if let Some(toast) = self.imp().toast.get() {
            toast.show_removable(description, on_undo);
        }
    }

    /// The list pane's placeholder for inbox zero, offline and sync failure.
    ///
    /// Canvas 3d. It sits *over* the message list rather than beside it and
    /// hides itself the moment there are rows, which is the seam
    /// `crate::list_state` was built for.
    pub fn list_state(&self) -> ListStateView {
        self.imp()
            .list_state
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Switch to `mailbox`, the way picking it in the sidebar does.
    ///
    /// The one way in from outside a click on an already-visible sidebar
    /// row — a notification's click, today. A no-op before
    /// [`install_feeds`](Self::install_feeds) has run, since there is
    /// nothing yet to switch to.
    pub fn open_mailbox(&self, mailbox: postio_model::ids::MailboxId) {
        let show = self.imp().open_mailbox.borrow().clone();
        if let Some(show) = show {
            self.sidebar().select(mailbox);
            show(mailbox);
        }
    }

    /// Switch to `mailbox` and put the keyboard on `message`, once its row
    /// is resident.
    ///
    /// See [`MessageListView::select_message`] for what "once" means: the
    /// mailbox has just been switched to, so nothing is resident yet, and
    /// the message is normally in the very first page that answers.
    pub fn open_message(
        &self,
        mailbox: postio_model::ids::MailboxId,
        message: postio_model::ids::MessageId,
    ) {
        self.open_mailbox(mailbox);
        self.list().select_message(message);
    }

    /// Feed both panes from the runtime, and wire the sidebar to the list.
    ///
    /// The one call whoever assembles the application makes: hand it the two
    /// sources and an account, keep the [`Feeds`] it returns, and give every
    /// [`postio_core::Event`] to [`Feeds::apply`].
    ///
    /// Picking a folder becomes a load of that folder here rather than in
    /// the sidebar, because the sidebar has no business knowing there is a
    /// message list — and because this is the one place that already holds
    /// both.
    pub fn install_feeds(
        &self,
        account: postio_model::ids::AccountId,
        address: &str,
        messages: std::rc::Rc<dyn MessageSource>,
        mailboxes: std::rc::Rc<dyn MailboxSource>,
    ) -> Feeds {
        let list = self.list();
        let feed = list.feed(messages.clone());
        let folders = Folders::new(&self.sidebar(), mailboxes);

        // One way to show a folder, whether the user picked it or the window
        // is opening on the one they were last in.
        let show: std::rc::Rc<dyn Fn(postio_model::ids::MailboxId)> = {
            let feed = feed.clone();
            let folders = folders.clone();
            let list = list.clone();
            std::rc::Rc::new(move |id| {
                if let Some(mailbox) = folders.mailbox(id) {
                    // The same word the sidebar uses, from the same place:
                    // the folder the user clicked must not change its name
                    // on the way to the header above the rows. Among its
                    // siblings, because a role's twin is named by the
                    // server, not by the role (#501).
                    list.set_mailbox(
                        &crate::sidebar::display_name(&mailbox, &folders.mailboxes()),
                        mailbox.counts.unread,
                    );
                }
                // The sidebar deals in row ids; everything below here deals
                // in scopes, because "Flagged" is a query and has no folder
                // to name.
                feed.open(folders.scope_of(id));
            })
        };
        *self.imp().open_mailbox.borrow_mut() = Some(show.clone());

        self.sidebar().connect_selected({
            let show = show.clone();
            move |id| show(id)
        });

        // The folders are not there yet when `open` returns, so the first
        // one to show is chosen when they arrive — the folder the window was
        // restored into, or the inbox. Opening into no folder at all would
        // be asking the user a question before saying hello.
        // `#` in the box jumps to a folder, and it can only offer folders it
        // has been told about.
        folders.connect_loaded({
            let finder = self.finder();
            move |mailboxes| finder.set_mailboxes(mailboxes)
        });
        // A picked label goes onto whatever the box was opened over. The
        // selection is resolved here rather than captured when the box
        // opened, for the reason `Move`'s own pick does it this way: the box
        // is modal over the list, so the selection cannot have moved, and
        // reading it now is one fewer piece of state to keep true.
        self.finder().connect_label(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |label| {
                // Through `press_escape`, not `close_finder` directly, so
                // that everyone who registered for a dismissal -- this
                // window's own focus restore, and #1011's search cleanup --
                // hears about it, whatever picked the finder is closing for.
                window.finder().press_escape();
                window.act(postio_core::Command::AddLabel {
                    target: postio_core::MessageTarget::Selection,
                    label: Some(label),
                    on: None,
                });
            }
        ));
        self.finder().connect_folder(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            show,
            move |id| {
                // A folder answers whichever question the box was opened
                // with. `m` asked where to move the selection; `#` asked
                // where to go.
                if window.imp().pending_move.take() {
                    // Still `Selection`, not the ids behind it: the rows the
                    // user marked have not moved while the box was up, and
                    // keeping the reference is what lets `Ctrl+A` then `m`
                    // move a whole mailbox as one command.
                    window.act(postio_core::Command::Move {
                        target: postio_core::MessageTarget::Selection,
                        to: Some(id),
                    });
                    return;
                }
                window.sidebar().select(id);
                show(id);
            }
        ));

        folders.connect_loaded({
            let show = show.clone();
            let feed = feed.clone();
            let folders = folders.clone();
            let sidebar = self.sidebar();
            // Which folder tree this handler has already opened something
            // for. `None` is "not yet": generations start at zero, so zero
            // is a real value rather than a spare one.
            //
            // This fires on *every* load, and picking a default every time
            // is what threw the list back to the inbox whenever a resync
            // emitted `MailboxesChanged` (#813).
            let picked_for = std::cell::Cell::new(None::<u64>);
            move |loaded| {
                let generation = folders.generation();
                if picked_for.get() == Some(generation) {
                    return;
                }
                // Something out of *this* tree is already on screen, so the
                // user or the caller has chosen it and this handler's turn
                // is spent. `ListScope::is_drawn_from` is the whole of the
                // judgement -- see its docs for why "is anything open" could
                // not answer it from either direction.
                if feed
                    .scope()
                    .is_some_and(|scope| scope.is_drawn_from(loaded))
                {
                    picked_for.set(Some(generation));
                    return;
                }
                // Only a *real* folder counts as a pick. The sidebar's
                // virtual rows carry sentinel ids (Flagged is -1), and
                // GtkListBox auto-selects the first row it gets — so on a
                // fresh account, whose folders arrive a beat after the
                // virtual rows, the sentinel would otherwise win here and
                // the window would open into an empty Flagged view instead
                // of the inbox. Caught by tests/e2e.rs in postio-app, the
                // first time anything drove a first sync into a real window.
                let picked = sidebar.selected().filter(|id| id.get() > 0);
                if let Some(id) = picked.or_else(|| folders.default_mailbox()) {
                    // Recorded only when something was actually opened. An
                    // account whose first read comes back empty has not had
                    // its turn yet, and must still get one when the folders
                    // arrive.
                    picked_for.set(Some(generation));
                    sidebar.select(id);
                    show(id);
                }
            }
        });

        // The list pane's named states read the same status the sidebar's
        // line does, so there is one connection and one answer about it —
        // and they also depend on whether there are rows, which arrive a
        // beat after the status does.
        folders.connect_status(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            folders,
            #[strong]
            feed,
            move |_| window.refresh_list_state(&folders, &feed)
        ));
        // And when the list is aimed somewhere else entirely. What the pane
        // says depends on *which* scope the rows came from -- an aggregate
        // answers ADR 0005 Q10's rule and a folder does not -- so a scope
        // change re-derives it. Neither of the other two triggers fires for
        // one: the connection has not moved, and switching to a view with
        // the same number of rows changes nothing about the model.
        feed.connect_opened(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            folders,
            #[strong(rename_to = opened_feed)]
            feed,
            move || window.refresh_list_state(&folders, &opened_feed)
        ));
        list.model().connect_items_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            folders,
            #[strong]
            feed,
            move |_, _, _, _| window.refresh_list_state(&folders, &feed)
        ));

        folders.open(account, address);
        *self.imp().messages.borrow_mut() = Some(messages);
        Feeds::new(feed, folders)
    }

    /// Re-derive the list pane's named state from `status`.
    ///
    /// `stored` and `queued` are what the local store still holds and what is
    /// waiting to reach the server. Neither has a cheap accessor on this side
    /// of the crate boundary yet — `postio-storage`'s operation queue has no
    /// count — so they are reported as what the pane can actually see, and
    /// `postio-qhz` will widen them when the counts exist.
    fn refresh_list_state(&self, folders: &Folders, feed: &Feed) {
        let rows = self.list().model().n_items() as u64;

        // An aggregate view answers by ADR 0005 Q10's rule instead of by the
        // single-account states: a whole-pane "Offline" would be a claim
        // about every account when only one of them is away. Named from the
        // sidebar's own list so the name in the banner and the hue on the
        // account's row are the same account in the same order.
        let drawn = matches!(feed.scope(), Some(postio_model::ListScope::Unified)).then(|| {
            let names = self.sidebar().account_names();
            folders
                .statuses()
                .into_iter()
                .filter_map(|(id, status)| {
                    let name = names
                        .iter()
                        .find(|(candidate, _)| *candidate == id)
                        .map(|(_, name)| name.clone())?;
                    Some((id, name, status))
                })
                .collect::<Vec<_>>()
        });
        // The same accounts, twice over, from one reading of one rule: the
        // banner names the ones the view cannot vouch for, and a whole-view
        // selection is scoped to the ones it can. Deriving those separately
        // is how the disclosure and the selection would come to disagree
        // about which account is which (#811, ADR 0005 Q10).
        self.list().set_reach(match &drawn {
            Some(accounts) => crate::selection::Reach {
                accounts: accounts
                    .iter()
                    .filter(|(_, _, status)| crate::list_state::is_current(status))
                    .map(|(id, _, _)| *id)
                    .collect(),
                omitted: accounts
                    .iter()
                    .filter(|(_, _, status)| !crate::list_state::is_current(status))
                    .map(|(_, name, _)| name.clone())
                    .collect(),
            },
            // Not an aggregate: one account, nothing left out.
            None => crate::selection::Reach::default(),
        });
        let aggregate = drawn.map(|accounts| {
            accounts
                .into_iter()
                .map(|(_, name, status)| (name, status))
                .collect::<Vec<_>>()
        });
        self.list_state().set_accounts(aggregate);
        self.list_state()
            .set_status(folders.status(), rows, rows, 0);
    }

    /// Say that the list is showing results for `query`, or a mailbox again.
    ///
    /// Told rather than inferred from the query box: the box stays up with
    /// the query still in it after `Esc` puts the folder back, so "there is
    /// text in the box" and "the list is showing that text's results" are
    /// different facts and only the second one belongs here.
    pub fn set_searching(&self, query: Option<&str>) {
        self.list_state().set_searching(query.map(str::to_owned));
    }

    fn build(&self) {
        self.set_title(Some("Postio"));
        self.add_css_class("postio-window");

        // Every window carries its own scheme classes: `tokens.css` keys its
        // dark and high-contrast blocks off `:root`, which in GTK is the root
        // *widget*, so the variables have to land here rather than on the
        // application. See `crate::style`.
        style::track(self);

        let shell = Shell::new();
        let sidebar = Sidebar::new();
        sidebar.set_vexpand(true);
        shell.sidebar().append(&sidebar);

        // The context follows the keyboard, however the keyboard got there.
        // Clicking a folder gives the row focus, and if the context did not
        // follow, `j` would scroll the message list while the focus ring sat
        // visibly on a folder — keys doing one thing and the screen saying
        // another, which is worse than either alone.
        let focus = gtk::EventControllerFocus::new();
        focus.connect_enter(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if window.context() != Context::Sidebar {
                    window.imp().before_sidebar.set(Some(window.context()));
                    window.set_context(Context::Sidebar);
                }
            }
        ));
        focus.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if window.context() == Context::Sidebar {
                    let previous = window.imp().before_sidebar.take();
                    window.set_context(previous.unwrap_or(Context::List));
                }
            }
        ));
        sidebar.add_controller(focus);

        // A message dragged onto a folder is the `m` key with the destination
        // already answered — the same registry command, so it is undoable the
        // same way and reaches the server through the same queue.
        sidebar.connect_dropped(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |dragged, mailbox| {
                let target = match dragged {
                    crate::list_view::Dragged::Messages(messages) => {
                        postio_core::MessageTarget::Messages(messages)
                    }
                    // A drag of the selection stays a *reference* to the
                    // selection all the way to the handler, so moving forty
                    // thousand messages is one command with one target and
                    // not forty thousand ids that had to be listed to be
                    // carried across the window.
                    crate::list_view::Dragged::Selection => postio_core::MessageTarget::Selection,
                };
                window.act(postio_core::Command::Move {
                    target,
                    to: Some(mailbox),
                });
            }
        ));

        // The named states cover the rows rather than replacing them: an
        // empty mailbox still has a header saying which mailbox it is, and
        // the state view hides itself the instant a row arrives.
        let list_view = MessageListView::new();
        let list_state = ListStateView::new();
        let list_overlay = gtk::Overlay::new();
        list_overlay.set_vexpand(true);
        list_overlay.set_child(Some(&list_view));
        list_overlay.add_overlay(&list_state);
        shell.list().append(&list_overlay);

        // ADR 0012 Q4: above the list rather than over it, so the rows it
        // is talking about stay visible and scrollable underneath — the
        // same arrangement `ListStateView`'s banner placement makes, and
        // for the same reason. Hidden until `postio-app` says otherwise.
        let orientation = crate::orientation::OrientationStrip::new();
        shell.list().prepend(&orientation.widget());
        orientation.connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.orientation().retire()
        ));
        let _ = self.imp().list_pane.set(list_overlay.clone());

        // The mouse runs the same commands the keyboard does, through the
        // same path: a button that acted directly would be a second
        // implementation of a verb the registry already owns.
        list_view.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |command| window.act(command)
        ));

        let header = header::build();

        // The toggle drives the sidebar, and the breakpoints drive the toggle:
        // widening the window past the three-pane threshold brings the sidebar
        // back, and the button has to say so.
        header.sidebar_toggle.connect_toggled(glib::clone!(
            #[weak]
            shell,
            move |toggle| shell.set_sidebar_visible(toggle.is_active())
        ));
        shell.connect_notify_local(
            Some("sidebar-visible"),
            glib::clone!(
                #[weak(rename_to = toggle)]
                header.sidebar_toggle,
                move |shell: &Shell, _| toggle.set_active(shell.sidebar_visible())
            ),
        );

        // The results hang under the header's field rather than replacing
        // the workspace: the canvas shows the panes still visible behind
        // them, and a surface that blanked the window would lose the context
        // the user is choosing in.
        let finder = Finder::new();
        finder.attach(&header.search);
        let cheatsheet = CheatSheet::new();
        cheatsheet.set_visible(false);
        let settings = SettingsPanel::new();
        settings.set_visible(false);
        // Canvas 3g. An overlay like the rest — a message's structure is
        // something to look at, not something the application has to stop for.
        // `Esc` reaches it through the registry's `Back`, handled generically
        // below wherever the panel is visible — the same path `Esc` takes
        // everywhere else.
        let parts = crate::parts::PartsPanel::new();
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&shell));
        overlay.add_overlay(&finder);
        overlay.add_overlay(&cheatsheet);
        overlay.add_overlay(&settings);
        overlay.add_overlay(&parts);
        let _ = self.imp().parts.set(parts);

        let layout = adw::ToolbarView::new();
        layout.add_top_bar(&header.bar);
        layout.set_content(Some(&overlay));

        // Outermost: a toast has to float over the header and the panes
        // alike, not just over whichever pane happened to trigger it.
        let toast = crate::toast::Toast::new();
        toast.overlay().set_child(Some(&layout));
        self.set_content(Some(toast.overlay()));

        // Alt-tabbing away stops the dwell (#71). Leaving Postio open on a
        // message while you do something else is not reading it, and a
        // machine left alone overnight must not come back with whatever the
        // cursor happened to be on marked read.
        // Both clocks, not just the list's: a conversation left open while
        // the user is elsewhere is no more "in front of them" than a list row
        // is (#945).
        self.connect_is_active_notify(|window| {
            if !window.is_active() {
                window.reader_now_shows(Showing::NothingInFrontOfAnybody);
            }
        });

        let undo = gio::SimpleAction::new("undo", None);
        undo.connect_activate(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_, _| window.run(CommandId::Undo)
        ));
        self.add_action(&undo);

        // Breakpoints only fire once the window has a size, so the restored
        // state goes on first and the breakpoints correct it if it does not
        // fit.
        self.restore(&shell, &sidebar);
        shell.install_breakpoints(self);
        header.sidebar_toggle.set_active(shell.sidebar_visible());

        let _ = self.imp().shell.set(shell);
        let _ = self.imp().sidebar.set(sidebar);
        let _ = self.imp().list_state.set(list_state);
        let _ = self.imp().list.set(list_view);
        let _ = self.imp().finder.set(finder);
        let _ = self.imp().cheatsheet.set(cheatsheet);
        let _ = self.imp().orientation.set(orientation);
        // The context follows the keyboard into the account list, the same
        // way it follows it into the folders — and scoped to that list
        // rather than to the panel, because the panel also holds a TextView
        // of the literal config.toml where `d` has to insert a `d`
        // (ADR 0005 Q6c). The TextView never enters this context, so the
        // trap is closed by construction rather than by remembering.
        let accounts_focus = gtk::EventControllerFocus::new();
        accounts_focus.connect_enter(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if window.context() != Context::Accounts {
                    window.imp().before_accounts.set(Some(window.context()));
                    window.set_context(Context::Accounts);
                }
            }
        ));
        accounts_focus.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if window.context() == Context::Accounts {
                    let previous = window.imp().before_accounts.take();
                    window.set_context(previous.unwrap_or(Context::List));
                }
            }
        ));
        settings.accounts_list().add_controller(accounts_focus);

        let _ = self.imp().settings.set(settings);
        let _ = self.imp().overlay.set(overlay);
        let _ = self.imp().compose_button.set(header.compose.clone());
        let _ = self.imp().toast.set(toast);
        self.imp().context.set(Some(Context::List));

        self.install_keyboard();
    }

    /// Builds the resolver from the registry defaults and starts listening.
    fn install_keyboard(&self) {
        let keymap = postio_core::Keymap::resolve(&Default::default());
        let (resolver, problems) = Resolver::from_commands(&keymap);
        report(&problems);
        self.settings().set_keymap_problems(&problems);
        let _ = self.imp().resolver.set(std::cell::RefCell::new(resolver));
        // The registry's own bindings, so the box and the cheat sheet print
        // keys from the first frame rather than from whenever `config.toml`
        // gets around to being read. `apply_keymap` replaces them if it
        // says something different.
        self.finder().set_keymap(keymap.clone());
        self.orientation().set_keymap(&keymap);
        self.cheatsheet().set_keymap(keymap);

        let finder = self.finder();
        finder.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| {
                // Through `press_escape`, for the reason `connect_label`
                // above gives (#1011).
                window.finder().press_escape();
                // Through `run_action`, not straight out: a command the
                // window answers itself means the same thing chosen from the
                // palette as it does typed, and the bus must not hear it
                // twice.
                window.run_action(id);
            }
        ));
        // Arriving somewhere is the end of asking where to go, so the box
        // gets out of the way — the same as running a command. Search is the
        // exception: its results *are* the message list, so the field stays
        // up with the query still in it.
        finder.connect_folder(glib::clone!(
            #[weak(rename_to = window)]
            self,
            // Through `press_escape`, not `close_finder` directly, for the
            // reason `CommandId::Back` gives (#1011).
            move |_| window.finder().press_escape()
        ));
        finder.connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close_finder()
        ));

        self.cheatsheet().connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close_cheatsheet()
        ));

        self.settings().connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close_settings()
        ));

        // Capture, not bubble: a single-key binding has to be seen before the
        // focused widget consumes it, and whether the focused widget *should*
        // consume it is the resolver's decision, not the propagation order's.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, state| window.handle_key(key, state)
        ));
        self.add_controller(keys);
    }

    /// Delivers one key press to the resolver and acts on what it resolves to.
    ///
    /// Public because it is the whole keyboard path in one call: the controller
    /// installed on this window is a two-line forwarder to it, and a test can
    /// press a key without synthesizing a GDK event, which GTK4 gives no
    /// supported way to do.
    pub fn handle_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
    ) -> glib::Propagation {
        self.resolve_key(key, state, self.key_context(), self.is_typing())
    }

    /// Delivers a key press that arrived in a satellite window.
    ///
    /// The detached composer is a real `GtkWindow` of its own, so its keys
    /// never reach this window's controller — but it must not grow a keymap
    /// of its own either, or `ctrl+s` would mean one thing in the pane and
    /// another in the window, and `[keys]` would only reach one of them. So
    /// the satellite forwards here: same resolver, same user bindings, same
    /// command registry. Only the two things that genuinely differ come from
    /// the caller — `context`, because this window has gone back to its own,
    /// and whether the user is typing, which is a fact about the *satellite's*
    /// focus and would otherwise be read off a widget nobody is looking at.
    pub fn handle_key_in(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        source: &impl IsA<gtk::Window>,
        context: Context,
    ) -> glib::Propagation {
        // Same two halves as `is_typing`, read off the satellite's focus
        // rather than this window's. `focused_field` walks up from its own
        // root, so it answers for whichever window the composer is in.
        let typing = gtk::prelude::GtkWindowExt::focus(source.as_ref())
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
            || self.composer_body_has_keyboard();
        self.resolve_key(key, state, KeyContext::from(context), typing)
    }

    fn resolve_key(
        &self,
        key: gtk::gdk::Key,
        state: gtk::gdk::ModifierType,
        context: KeyContext,
        typing: bool,
    ) -> glib::Propagation {
        let Some(chord) = keymap::Chord::from_key_event(key, state) else {
            return glib::Propagation::Proceed;
        };
        let Some(resolver) = self.imp().resolver.get() else {
            return glib::Propagation::Proceed;
        };

        let outcome =
            resolver
                .borrow_mut()
                .press(&chord, context, typing, std::time::Instant::now());

        match outcome {
            // Not a built-in parse any more: a key can be bound to a
            // registered command too, and the id the resolver carries is a
            // string precisely so this layer decides what it names.
            Outcome::Command(id) => match id.parse::<ActionId>() {
                Ok(id) => {
                    self.run_action(id);
                    glib::Propagation::Stop
                }
                // A binding for a command this build does not know — a
                // `[keys]` entry written by a newer Postio, or an extension
                // that never loaded: leave the key alone rather than
                // swallowing it.
                Err(_) => glib::Propagation::Proceed,
            },
            // A half-typed sequence is consumed so its first chord does not also
            // reach the widget underneath.
            Outcome::Pending(_) => glib::Propagation::Stop,
            Outcome::Unhandled => {
                // The silent path, and the one postio-73 was reported from:
                // a key that does nothing, with nothing said about why. All
                // three inputs the resolver used are here, so "it randomly
                // stopped working" becomes one line naming which of them it
                // was. No message content -- a chord, a context and a widget
                // type name are not mail.
                tracing::debug!(
                    chord = %chord,
                    ?context,
                    typing,
                    focus = focused_type(self).as_deref().unwrap_or("none"),
                    finder_open = self.finder().is_open(),
                    finder_has_keyboard = self.finder().has_keyboard(),
                    "key resolved to nothing"
                );
                glib::Propagation::Proceed
            }
        }
    }

    /// Acts on the commands that are about the window, and passes on the rest.
    /// A keystroke says only which verb it meant, so the registry's default
    /// target — "the selection" — is the whole of the invocation.
    ///
    /// Everything else is [`Window::act`]'s, so the key and the click that
    /// mean the same thing take the same path rather than two that have to be
    /// kept in step.
    fn run(&self, id: CommandId) {
        self.act(postio_core::Command::default_for(id));
    }

    /// Acts on any action a surface resolved to, built-in or registered.
    ///
    /// A built-in becomes the invocation its registry default implies. A
    /// registered command has no such invocation — `Command` is the closed
    /// vocabulary and `default_for` cannot answer for an id this build has
    /// never seen — so it goes out through
    /// [`connect_ext_command`](Self::connect_ext_command) instead, for
    /// whoever owns the bus to dispatch.
    ///
    /// This is where the two vocabularies stay distinguishable to the
    /// compiler while being equal to the user, which is the whole shape of
    /// ADR 0002.
    fn run_action(&self, id: ActionId) {
        // ADR 0012 Q6: the first-run orientation is over the moment somebody
        // runs a command from the keyboard or the palette, whether or not it
        // ever appeared. This is the seam that can tell that apart from a
        // click: `connect_action` sees only the commands the window passes
        // *out*, so it never sees `j` -- which is the exact gesture the ADR
        // names -- and `act` sees the mouse's invocations too, which it says
        // are not evidence of anything.
        self.orientation().retire();
        match id {
            ActionId::Builtin(id) => self.run(id),
            ActionId::Ext(id) => {
                for handler in self.imp().ext_commands.borrow().iter() {
                    handler(id);
                }
            }
        }
    }

    /// Leaves the conversation, and says what the application should be told
    /// if it did.
    ///
    /// `None` means this was not a way out of the conversation and the
    /// caller's own command stands. Unlike `handled_here` this acts *and*
    /// lets the command go out: the keyboard moves here, and `AppState` has
    /// to hear about it or its back stack and its keyboard context drift out
    /// of step with what is on screen.
    ///
    /// Used to swap the list column for a thread column (#1003). There is no
    /// column any more, so leaving a conversation is not a pane swap: the
    /// list never went anywhere, and what `Esc` and `h` do is give it the
    /// keyboard back.
    fn leave_conversation(&self, command: &postio_core::Command) -> Option<postio_core::Command> {
        if !matches!(
            command,
            postio_core::Command::Back | postio_core::Command::PrevView
        ) || self.context() != Context::Conversation
        {
            return None;
        }
        self.list().grab_focus();
        self.set_context(Context::List);
        Some(command.clone())
    }

    /// Whether the window answered `id` itself.
    ///
    /// Closing an overlay and moving the cursor are the window's own
    /// business: nothing outside it needs to hear about them, and there is
    /// nothing for a command bus to do with them.
    fn handled_here(&self, id: CommandId) -> bool {
        match id {
            CommandId::CommandPalette => self.open_finder(Mode::Command),
            CommandId::CheatSheet => self.toggle_cheatsheet(),
            // The conversation's own axis. `j`/`k` move between threads in
            // the list; these move between the messages of the one that is
            // open, and both are the window's business rather than the
            // bus's (#1007).
            CommandId::NextInConversation => {
                self.conversation().focus_next();
            }
            CommandId::PrevInConversation => {
                self.conversation().focus_previous();
            }
            CommandId::ToggleFold => {
                self.conversation().toggle_fold();
            }
            // Reader view is per message, so this is per message too: it acts
            // on whichever reader is currently drawing one, and does nothing
            // when that reader is already showing the sender's own markup
            // (#1009).
            CommandId::ViewOriginal => {
                self.reader_showing().view_original();
            }

            // The conversation's own, so it goes to the pane rather than out
            // on the bus: nothing outside this window has anything to do with
            // how much of a conversation is open (#1004).
            CommandId::ExpandAll => {
                self.conversation().expand_all();
            }
            CommandId::Settings => self.toggle_settings(),
            CommandId::Search => self.open_finder(Mode::Search),
            // The header button already flips this property directly
            // (`window.rs`, `sidebar_toggle.connect_toggled`); this is the
            // same call from the palette and `Ctrl+B`, which had no arm here
            // at all — see #756.
            CommandId::ToggleSidebar => {
                let shell = self.shell();
                shell.set_sidebar_visible(!shell.sidebar_visible());
            }
            // One `Esc` closes one overlay, nearest first — and a selection
            // is the nearest thing of all once every overlay is shut. It is
            // also the only way out of one that does not require picking a
            // row, which matters most when the selection is a predicate.
            // Nearest first. The parts panel is the innermost thing `Esc`
            // could mean while it is up.
            // The account row the keyboard is on. `focused_account` answers
            // `None` when the focus is elsewhere in the panel, and that is a
            // real answer: falling back to "the first account" would remove
            // somebody's mail on a keystroke aimed at nothing (ADR 0005 Q6c).
            CommandId::RemoveAccount => {
                if let Some(id) = self.settings().focused_account() {
                    self.settings()
                        .request_account_action(id, crate::settings::AccountAction::Remove);
                }
            }
            CommandId::UpdateCredential => {
                if let Some(id) = self.settings().focused_account() {
                    self.settings().request_account_action(
                        id,
                        crate::settings::AccountAction::UpdateCredential,
                    );
                }
            }
            CommandId::ToggleAccountEnabled => {
                if let Some(id) = self.settings().focused_account() {
                    self.settings().toggle_account_enabled(id);
                }
            }
            CommandId::RebuildAccountIndex => {
                if let Some(id) = self.settings().focused_account() {
                    self.settings()
                        .request_account_action(id, crate::settings::AccountAction::RebuildIndex);
                }
            }
            // Same shape and the same reason as the three above: the target
            // is the row the keyboard is on, and `focused_account` answering
            // `None` is a real answer. Falling back to "the first account"
            // would be the implicit specialness #960 exists to replace.
            CommandId::SetDefaultAccount => {
                if let Some(id) = self.settings().focused_account() {
                    self.settings()
                        .request_account_action(id, crate::settings::AccountAction::SetDefault);
                }
            }
            // `u` here means the removal toast, never the global stack: the
            // stack never held this removal (#464 wired it straight to
            // AccountRepository::restore), so nothing else could answer it.
            // Handled whether or not a toast is up -- falling through would
            // undo the last *mail* action from a context where the person is
            // looking at accounts.
            CommandId::Undo if self.context() == Context::Accounts => {
                if let Some(toast) = self.imp().toast.get() {
                    toast.activate_undo();
                }
            }
            CommandId::Back if self.parts().is_visible() => self.close_parts(),
            CommandId::Back if self.cheatsheet().is_visible() => self.close_cheatsheet(),
            // Through `press_escape`, not `close_finder` directly (#1011):
            // this is the path that runs once the keyboard has moved off
            // the search entry onto the list to read a result, and
            // `close_finder` alone restores focus and the keymap context
            // without ever telling `Feed` the search is over.
            CommandId::Back if self.finder().is_open() => self.finder().press_escape(),
            CommandId::Back if self.settings().is_visible() => self.close_settings(),
            // Nearer than a selection made before the keyboard went to the
            // folders: `Esc` in the sidebar means "back to the messages".
            CommandId::Back if self.context() == Context::Sidebar => self.leave_sidebar(),
            CommandId::Back if !self.list().selection().is_empty() => self.list().clear_selection(),

            // Where the keyboard is, and what an action would hit. Two
            // different things, moved by two different sets of keys — see
            // `crate::selection`.
            // `j`/`k` walk threads in the list, and only there. Walking the
            // messages *inside* an open conversation is `J`/`K`, which is a
            // different pair of bindings on a different surface (#1007).
            CommandId::NextMessage => self.list().next_row(),
            CommandId::PrevMessage => self.list().prev_row(),
            CommandId::FirstMessage => self.list().first_row(),
            CommandId::LastMessage => self.list().last_row(),
            CommandId::ToggleSelection => self.list().toggle_cursor_row(),
            CommandId::ExtendSelectionDown => self.list().extend_down(),
            CommandId::ExtendSelectionUp => self.list().extend_up(),
            CommandId::SelectAll => self.list().select_all(),

            // The folders. `j`/`k` reach these only in `Context::Sidebar`,
            // which is why the sidebar had to become a real context rather
            // than a focus flag — see `postio-cfd.2`.
            CommandId::FocusSidebar => self.enter_sidebar(),
            CommandId::CyclePane => self.cycle_pane(true),
            CommandId::CyclePaneBack => self.cycle_pane(false),
            CommandId::NextFolder => {
                self.sidebar().step(1);
            }
            CommandId::PrevFolder => {
                self.sidebar().step(-1);
            }
            CommandId::ToggleFolder => self.sidebar().toggle_focused(),
            // `g a`: cycle the strip's own account scope, unified then each
            // account in turn (#765). Selecting the row is the whole of it
            // -- the strip's `connect_row_selected` is what actually
            // re-points the folder feed on a real click, and this walks the
            // same rows the same way, so nothing downstream has to learn a
            // second path exists.
            CommandId::NextScope => self.sidebar().select_next_scope(),

            // The parts panel. Reached through `Context::Parts` for the same
            // reason the folders are — see `postio-14b`. Set and cleared by
            // `open_parts`/`close_parts`, the one door in and out of it.
            CommandId::OpenParts => self.reader().request_parts(),
            CommandId::NextPart => self.parts().next_part(),
            CommandId::PrevPart => self.parts().prev_part(),
            CommandId::OpenPart => self.parts().open_part(),
            CommandId::SavePart => self.parts().save_part(),
            CommandId::SaveAllParts => self.parts().save_all(),
            CommandId::OpenPartExternally => self.parts().open_externally(),
            CommandId::RenderPartOnce => self.parts().render_once(),

            // Scrolling the pane without moving the keyboard off the list
            // (#438) is the reader's own business the same way the parts
            // panel's cursor is -- nothing outside this window needs to hear
            // about it.
            CommandId::ScrollReaderDown => self.reader().page_down(),
            CommandId::ScrollReaderUp => self.reader().page_up(),
            _ => return false,
        }
        true
    }

    /// Put the keyboard in the folder list.
    ///
    /// Remembers where it came from, so leaving restores it. Does nothing
    /// when there are no folders yet: sending the keyboard into an empty
    /// pane on a first run would strand it somewhere with no rows to move
    /// between and nothing to say why.
    fn enter_sidebar(&self) {
        if self.context() == Context::Sidebar {
            return;
        }
        // A hidden sidebar has to come back before it can take the keyboard,
        // or the command would silently do nothing at the narrow breakpoint.
        if !self.shell().sidebar_visible() {
            self.shell().set_sidebar_visible(true);
        }
        if !self.sidebar().focus_folders() {
            return;
        }
        self.imp().before_sidebar.set(Some(self.context()));
        self.set_context(Context::Sidebar);
    }

    /// Move the keyboard one pane along: sidebar, list, reader, round.
    ///
    /// #494: bare Tab had no entry in the table at all, so its top-level
    /// meaning was whatever GTK's native focus chain produced -- "sometimes
    /// it changes panes, sometimes it changes items within a pane". This is
    /// the deliberate version.
    ///
    /// Three panes, always the same three. The drill-in used to make the
    /// middle one sometimes a thread column instead of the list (#1003);
    /// the list is only ever the list now, and the conversation is what the
    /// reading pane holds rather than a pane of its own.
    fn cycle_pane(&self, forward: bool) {
        let next = match (self.context(), forward) {
            (Context::Sidebar, true) => Context::List,
            (Context::List | Context::Conversation, true) => Context::Reader,
            (Context::Reader, true) => Context::Sidebar,
            (Context::Sidebar, false) => Context::Reader,
            (Context::List | Context::Conversation, false) => Context::Sidebar,
            (Context::Reader, false) => Context::List,
            // Tab does not resolve to this command anywhere else -- see
            // `PANE_SURFACES` -- so any other context means the keymap and
            // the registry disagree. Do nothing rather than guess a pane.
            _ => return,
        };
        self.focus_pane(next);
    }

    /// Put the keyboard in `pane`, and record the context that now owns it.
    fn focus_pane(&self, pane: Context) {
        match pane {
            // Reuses the sidebar's own entry path, which brings a hidden
            // sidebar back before focusing it -- otherwise the cycle would
            // silently skip a pane at the narrow breakpoint (#494's
            // acceptance says handled the same way `FocusSidebar` does).
            Context::Sidebar => self.enter_sidebar(),
            // The conversation is inside the reading pane, so the keyboard
            // going there is the reading pane taking it.
            Context::Conversation => {
                self.reader().view().grab_focus();
                self.set_context(Context::Conversation);
            }
            Context::List => {
                self.list().grab_focus();
                self.set_context(Context::List);
            }
            Context::Reader => {
                self.reader().view().grab_focus();
                self.set_context(Context::Reader);
            }
            _ => {}
        }
    }

    /// Give the keyboard back to whatever had it before the folders.
    fn leave_sidebar(&self) {
        let previous = self.imp().before_sidebar.take().unwrap_or(Context::List);
        self.set_context(previous);
        self.list().grab_focus();
    }

    /// Hand one invocation to everything listening, in both shapes.
    ///
    /// Every gesture goes out exactly once, and it goes out *whole*. The two
    /// seams are two views of the same invocation rather than two paths a
    /// command might take: `connect_command` consumers act on the verb alone
    /// — the composer opens, the editor launches — while `connect_action`
    /// consumers need to know what it was aimed at.
    ///
    /// The mouse used to reach the id seam through the window's fallthrough
    /// *and* the action seam with its own target, which handed a command bus
    /// subscribed to both one gesture as two different invocations: archive
    /// the selection, then archive the hovered row.
    fn deliver(&self, command: postio_core::Command) {
        let id = command.id();
        for handler in self.imp().commands.borrow().iter() {
            handler(id);
        }
        for handler in self.imp().actions.borrow().iter() {
            handler(command.clone());
        }
    }

    /// Whether the focused widget takes text.
    ///
    /// The other half of the "typing always wins" rule: the resolver decides
    /// *which* bindings survive a text field, and this decides whether it is
    /// being asked from inside one.
    fn is_typing(&self) -> bool {
        gtk::prelude::GtkWindowExt::focus(self)
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>())
            || self.composer_body_has_keyboard()
    }

    /// Whether the composer's body has the keyboard.
    ///
    /// The other half of [`is_typing`](Self::is_typing), and it cannot be
    /// answered by a type test. The body is a `WebView` over a
    /// `contenteditable` document rather than a `GtkText`, so the widget test
    /// above says "not typing" in the one field where a person is doing
    /// nothing else -- which is #602: `e` ran *reply* instead of typing an
    /// `e`, and a half-written reply answered itself.
    ///
    /// Not "the focus is a `WebView`": the reader is one too, and `e` has to
    /// keep meaning reply while reading. What separates them is that the body
    /// is editable, and `Composer::focused_field` is where that distinction
    /// already lives -- asked rather than restated, so the two cannot drift.
    ///
    /// Asks the slot, not [`Window::composer`], which would *install* a
    /// composer to be told there is not one.
    fn composer_body_has_keyboard(&self) -> bool {
        self.imp()
            .composer
            .get()
            .is_some_and(|composer| composer.focused_field() == Some(crate::composer::Field::Body))
    }

    fn key_context(&self) -> KeyContext {
        // The box owns the keyboard while it *has* the keyboard, and which of
        // its two contexts depends on the mode: `Enter` runs a command in one
        // and searches in the other.
        //
        // Not "while it is open". A search deliberately leaves the field up
        // with the query still in it, so `is_open` stays true while the user
        // is back in the message list — and asking it here pinned the
        // resolver to `Search` from the first search onwards, silently
        // killing every single-key binding for the rest of the session. See
        // `Finder::has_keyboard` and `postio-73`.
        let finder = self.finder();
        match finder.context().filter(|_| finder.has_keyboard()) {
            Some(context) => KeyContext::from(context),
            None => KeyContext::from(self.context()),
        }
    }
}

/// The type name of whatever holds the keyboard, for a log line.
///
/// The type rather than the widget: it is enough to tell a `GtkText` the user
/// forgot they were in from the message list, and it cannot carry anything a
/// widget is displaying.
fn focused_type(window: &Window) -> Option<String> {
    gtk::prelude::GtkWindowExt::focus(window).map(|focus| focus.type_().name().to_string())
}

/// What the key resolver could not make sense of.
///
/// `debug`, not `warn`: every one of these is a binding the *user* wrote that
/// the resolver dropped, the application carries on with the rest, and the
/// settings panel shows the same problems where they can be fixed. A level
/// that fires on somebody's half-edited `config.toml` is not a level anyone
/// keeps reading.
fn report(problems: &[String]) {
    for problem in problems {
        tracing::debug!(problem, "keymap");
    }
}

impl Window {
    /// The one box: search mail, run a command, jump to a folder.
    pub fn finder(&self) -> Finder {
        self.imp()
            .finder
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Which surface owns the keyboard.
    pub fn context(&self) -> Context {
        self.imp().context.get().unwrap_or(Context::List)
    }

    /// Tells the window which surface owns the keyboard.
    ///
    /// The panes call this as focus moves; it is what makes `Esc` mean
    /// something different in a thread than in the list.
    pub fn set_context(&self, context: Context) {
        self.imp().context.set(Some(context));
        // The box filters its commands by the context it was opened *over*,
        // never by the one it owns while it is open. Forwarding this while it
        // is up would empty it the instant it appeared: `Context::Search` has
        // no message actions in it, which is the whole point of contexts.
        if !self.finder().is_open() {
            self.finder().set_context(context);
        }
        // The sheet answers "what can I do now", so it tracks the context
        // whether or not the box is up: `?` is pressed from wherever the
        // reader is stuck (#182).
        self.cheatsheet().set_context(context);
    }

    /// What the mail on screen belongs to.
    ///
    /// Reaches the two surfaces that filter by it — the palette and the
    /// cheat sheet — so a unified view offers no `Move` in either. The
    /// registry decides; this is only how the answer gets there.
    pub fn set_scope(&self, scope: postio_core::Scope) {
        self.imp().scope.set(scope);
        self.finder().set_scope(scope);
        self.cheatsheet().set_scope(scope);
    }

    /// The scope the window is showing.
    pub fn scope(&self) -> postio_core::Scope {
        self.imp().scope.get()
    }

    /// Called with every command a key press resolves to.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().commands.borrow_mut().push(Box::new(handler));
    }

    /// Called with the keymap whenever one is applied, and once immediately
    /// with the keymap already in force.
    ///
    /// [`apply_keymap`](Self::apply_keymap) reaches the surfaces this window
    /// owns directly. The search column is not one of them — `postio-app`
    /// builds it and hands it the shell — and its footer names keys too, so
    /// it needs a way to hear about a rebind (#828). Called immediately as
    /// well as on change, because a surface attached after the first
    /// `apply_keymap` would otherwise keep the registry defaults until the
    /// user next edited `config.toml`.
    pub fn connect_keymap(&self, handler: impl Fn(&postio_core::Keymap) + 'static) {
        if let Some(keymap) = self.imp().keymap.borrow().as_ref() {
            handler(keymap);
        }
        self.imp().keymaps.borrow_mut().push(Box::new(handler));
    }

    /// Called with the new `[storage] max_bytes` every time `config.rs`'s
    /// reload loop sees that section move (#929).
    ///
    /// `postio-gtk` has no store to enforce a ceiling against, so this only
    /// asks — the composition root, which owns the `Database`/`BlobStore`
    /// pair, is what subscribes and re-runs the eviction pass.
    ///
    /// Not replayed on connect the way [`connect_keymap`](Self::connect_keymap)
    /// is: the initial ceiling is already read once at startup through
    /// `Wiring::storage_ceiling`, and this signal only exists for the values
    /// after that.
    pub fn connect_storage_changed(&self, handler: impl Fn(Option<u64>) + 'static) {
        self.imp()
            .storage_changed
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Fires what [`connect_storage_changed`](Self::connect_storage_changed)
    /// is listening for. Called from `config.rs`'s reload loop, in the same
    /// crate.
    pub(crate) fn notify_storage_changed(&self, max_bytes: Option<u64>) {
        for handler in self.imp().storage_changed.borrow().iter() {
            handler(max_bytes);
        }
    }

    /// Called with every *registered* command a key or a palette row reaches.
    ///
    /// The extension counterpart of [`connect_action`](Self::connect_action).
    /// Separate rather than folded in because the two carry different things:
    /// a built-in arrives as a `Command` with the registry's default target
    /// resolved, and a registered command arrives as an id with no payload at
    /// all, because nothing in this build knows what payload it would have.
    ///
    /// The application subscribes here and routes to
    /// `Dispatcher::dispatch_ext`.
    pub fn connect_ext_command(&self, handler: impl Fn(postio_core::ExtId) + 'static) {
        self.imp().ext_commands.borrow_mut().push(Box::new(handler));
    }

    /// Called with every *invocation* — a command and what it is aimed at.
    ///
    /// This is the seam a command bus subscribes to, and it sees every
    /// gesture: the keyboard's invocations carry the registry's default
    /// target, because a keystroke says only which verb it meant and "the
    /// selection" is the right answer for one; the mouse's carry their own,
    /// because a hover action names its row and a drop names its destination
    /// folder. Both are the same verbs from the same table, so a subscriber
    /// here never has to ask which input produced one.
    ///
    /// [`connect_command`](Self::connect_command) is the other view of the
    /// same invocation, for consumers that only need the verb. Subscribing to
    /// both would see each gesture twice.
    pub fn connect_action(&self, handler: impl Fn(postio_core::Command) + 'static) {
        self.imp().actions.borrow_mut().push(Box::new(handler));
    }

    /// Run an invocation: the window's own commands first, then the handlers.
    pub fn act(&self, command: postio_core::Command) {
        // A move with no destination is half a request: `None` means "ask the
        // user", and this is the window asking. Matched on the whole command
        // rather than its id because the *answered* move — from a drop, or
        // from the pick below — is the same id and has to go straight out.
        if matches!(command, postio_core::Command::Move { to: None, .. }) {
            self.open_finder(Mode::Mailbox);
            self.imp().pending_move.set(true);
            return;
        }
        // The same shape for a label (#780, ADR 0005): `None` means "ask",
        // and an answered `AddLabel` -- from the pick below -- carries its
        // label and goes straight out. Without this the command reaches the
        // dispatcher, which rejects it with "Pick a label to add", and the
        // menu item is the dead end #766 removed it for being.
        if matches!(command, postio_core::Command::AddLabel { label: None, .. }) {
            self.open_finder(Mode::Label);
            return;
        }
        // Opening a message is navigation, not a store verb: it changes what
        // is on screen and nothing about the mail. The bus owns the verbs
        // that write — archive, flag, move, snooze — and this is answered
        // here beside the other view commands, the way `PrevView` and
        // `NextScope` are.
        //
        // It went unanswered anywhere for a while (#767). Nothing noticed
        // because the *keyboard* path never needed it: `Return` on a row
        // reaches `connect_activated` through `GtkListView`'s own action, so
        // reading mail worked and only the paths that name a message were
        // dead — the search preview's `Ret` and its `Open` button, which
        // sent this command and got a dispatcher rejection back.
        //
        // Matched on the whole command rather than its id, like the move
        // above, because which message is being asked for is the substance:
        // `Some` names one and `None` means the row the cursor is on.
        if let postio_core::Command::OpenMessage { message } = command {
            let list = self.list();
            match message {
                // Put the cursor on it, then *activate* it — do not rely on
                // the cursor having moved. A result opened out of the search
                // preview is usually the row the list already had under the
                // cursor, and a move that moves nowhere emits nothing, so an
                // open that waited for `notify::selected` would do nothing
                // for exactly the commonest case (#601 is the same shape,
                // for a click).
                //
                // When the row is not resident yet, `select_message` lands
                // the cursor asynchronously and the ordinary cursor path
                // fills the pane when it arrives; activating now would
                // activate whatever the cursor is on in the meantime, which
                // is why this asks first.
                Some(message) => {
                    list.select_message(message);
                    if list.cursor_id() == Some(message) {
                        list.activate_cursor();
                    }
                }
                None => list.activate_cursor(),
            }
            return;
        }
        // A command the window answers itself — closing an overlay, moving
        // the cursor — means the same thing however it was invoked, and stops
        // here either way.
        if self.handled_here(command.id()) {
            return;
        }
        let command = self.leave_conversation(&command).unwrap_or(command);
        self.deliver(command);
    }

    /// Runs `query` as though it had been typed and answered immediately,
    /// rather than after the debounce a keystroke would wait out.
    ///
    /// The seam a saved search activates through (issue #10): the sidebar
    /// deals in query strings, not in typing, and a row that took 300ms to
    /// answer would look broken next to every other one that opens at once.
    pub fn run_search(&self, query: &str) {
        self.open_finder(Mode::Search);
        let finder = self.finder();
        finder.set_query(Query {
            mode: Mode::Search,
            text: query.to_string(),
        });
        if let Some(live) = finder.live() {
            live.flush();
        }
    }

    /// Opens the box in `mode`, remembering what to come back to.
    ///
    /// No animation and no dialog: the field is typeable the instant it has
    /// the keyboard, which is what the canvas means by search being
    /// navigation rather than a mode you enter.
    pub fn open_finder(&self, mode: Mode) {
        let finder = self.finder();
        // Whatever the last opening was for, this one is not it yet. Cleared
        // here rather than on close so that the pick which *closes* the box
        // can still see what the box was for — and so an abandoned move
        // cannot be answered by an unrelated `#` jump later.
        self.imp().pending_move.set(false);
        if !finder.is_open() {
            self.close_cheatsheet();
            self.close_settings();
            // Remembered before anything moves, so `Esc` puts the keyboard
            // back where the user left it rather than wherever the box
            // happened to leave it.
            self.imp()
                .before_finder
                .set(Some((self.context(), self.shell().focused_pane())));
        }
        finder.set_context(self.context());
        finder.open(mode);
        self.set_context(mode.context());
    }

    /// Closes the box and restores the view it opened over.
    pub fn close_finder(&self) {
        let finder = self.finder();
        if !finder.is_open() {
            return;
        }
        finder.close();
        if let Some((context, pane)) = self.imp().before_finder.take() {
            self.set_context(context);
            self.shell().set_focused_pane(pane);
        }
        // `pane` only ever names List or Reader (`postio-cfd.2`): it
        // predates the sidebar becoming a keyboard context at all, and has
        // no shape to record which row on it had the keyboard. So a search
        // opened by stepping onto a saved search (or by symptom 1 of #614)
        // is restored the same way `enter_sidebar` reaches the sidebar in
        // every other path, rather than trusted to whichever descendant
        // `shell().grab_focus()`'s own memory happens to restore.
        if self.context() == Context::Sidebar {
            self.sidebar().focus_folders();
        } else {
            self.shell().grab_focus();
        }
    }

    /// The `?` overlay.
    pub fn cheatsheet(&self) -> CheatSheet {
        self.imp()
            .cheatsheet
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// ADR 0012's first-run keyboard orientation, above the message list.
    pub fn orientation(&self) -> crate::orientation::OrientationStrip {
        self.imp()
            .orientation
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Shows the cheat sheet, or hides it if it is already up.
    ///
    /// Toggling rather than only opening: `?` is what the user pressed to get
    /// here, and a sheet its own key cannot close is one people get stuck in.
    pub fn toggle_cheatsheet(&self) {
        if self.cheatsheet().is_visible() {
            self.close_cheatsheet();
        } else {
            self.open_cheatsheet();
        }
    }

    /// Shows the cheat sheet over the workspace.
    pub fn open_cheatsheet(&self) {
        // Two overlays at once is one too many. Through `press_escape`, not
        // `close_finder` directly, for the reason `CommandId::Back` above
        // gives (#1011).
        self.finder().press_escape();
        let sheet = self.cheatsheet();
        sheet.set_visible(true);
        sheet.grab_focus();
    }

    /// Hides the cheat sheet.
    pub fn close_cheatsheet(&self) {
        self.cheatsheet().set_visible(false);
        self.shell().grab_focus();
    }

    /// The settings panel: canvas 3f, `config.toml` edited in place.
    pub fn settings(&self) -> SettingsPanel {
        self.imp()
            .settings
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Shows the settings panel over the workspace.
    pub fn open_settings(&self) {
        self.ensure_keys_focus_controller();
        // Only one overlay at a time. Through `press_escape`, not
        // `close_finder` directly, for the reason `CommandId::Back` above
        // gives (#1011).
        self.finder().press_escape();
        self.close_cheatsheet();
        // Read fresh on every open rather than cached, the same reason
        // `new_reader` loads its own copy each time rather than keeping one
        // long-lived: whatever the reader last wrote should show up here
        // without this panel needing to watch the file (#871).
        if let Some(path) = self.imp().allowlist_path.borrow().clone() {
            self.settings().set_remote_image_allowlist(
                crate::reader::RemoteImageAllowList::load_from(&path),
                path,
            );
        }
        self.settings().set_visible(true);
        self.settings().grab_focus();
    }

    /// Hides the settings panel and gives the keyboard back to the workspace.
    pub fn close_settings(&self) {
        self.settings().set_visible(false);
        self.shell().grab_focus();
    }

    /// Adds `keys_list`'s `Context::Keys` `EventControllerFocus`, the
    /// first time settings is actually opened — never during `Window::new`.
    ///
    /// `docs/engineering-notes.md`'s note on `SettingsPanel::build()` (#873,
    /// #880, #881) explains why nothing that touches a settings sub-widget
    /// gets added unconditionally during `Window::new`'s own construction:
    /// a full-`gtk_suite` corruption was chased there before, and even
    /// though it turned out to be an unrelated pre-existing flake in that
    /// specific case, the precautionary principle stands. `accounts_focus`,
    /// right above where this used to sit before that investigation, is the
    /// same shape and has not been observed to cause a problem — which is
    /// not the same as proof it cannot, so this stays deferred rather than
    /// treating that as precedent for adding a second one unconditionally.
    fn ensure_keys_focus_controller(&self) {
        if self.imp().keys_focus_installed.get() {
            return;
        }
        self.imp().keys_focus_installed.set(true);

        let keys_focus = gtk::EventControllerFocus::new();
        keys_focus.connect_enter(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if window.context() != Context::Keys {
                    window.imp().before_keys.set(Some(window.context()));
                    window.set_context(Context::Keys);
                }
            }
        ));
        keys_focus.connect_leave(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |_| {
                if window.context() == Context::Keys {
                    let previous = window.imp().before_keys.take();
                    window.set_context(previous.unwrap_or(Context::List));
                }
            }
        ));
        self.settings().keys_list().add_controller(keys_focus);
    }

    /// Shows the settings panel, or hides it if it is already up.
    pub fn toggle_settings(&self) {
        if self.settings().is_visible() {
            self.close_settings();
        } else {
            self.open_settings();
        }
    }

    /// Rebuilds the keymap after `config.toml` changed, without a restart.
    ///
    /// Everything downstream follows from this one call: the resolver reparses
    /// its chords, and the palette and the cheat sheet reprint their keys.
    pub fn apply_keymap(&self, keymap: postio_core::Keymap) {
        if let Some(resolver) = self.imp().resolver.get() {
            let problems = resolver.borrow_mut().apply_commands(&keymap);
            report(&problems);
            self.settings().set_keymap_problems(&problems);
        }
        self.finder().set_keymap(keymap.clone());
        self.cheatsheet().set_keymap(keymap.clone());
        self.orientation().set_keymap(&keymap);
        self.parts().set_keymap(&keymap);
        self.reader().set_keymap(&keymap);
        // Every message in the stack carries its own Reply/Reply all/Forward
        // caps now (#1002), so the pane has to be re-capped like every other
        // surface that shows a key.
        self.conversation().set_keymap(&keymap);
        // #828: the composer's Send / Schedule / Save draft hints were
        // literals, so they went on naming the default key after a rebind.
        //
        // Only if one exists. `composer()` builds the composer on first ask,
        // deliberately — a window used only for a test of the sidebar has no
        // reason to pay for one — and reaching for it here would build a
        // WebKit editor in every window that ever applies a keymap. A
        // composer made later picks the keymap up from `imp().keymap` at
        // construction instead.
        if let Some(composer) = self.imp().composer.get() {
            composer.set_keymap(&keymap);
        }
        for handler in self.imp().keymaps.borrow().iter() {
            handler(&keymap);
        }
        *self.imp().keymap.borrow_mut() = Some(keymap);
    }

    /// Apply the `[ui]` block: what the list shows, and how much of it.
    ///
    /// Live — the config watcher calls this on every save that changes the
    /// section, so turning hover actions off in `config.toml` takes them off
    /// the rows already on screen rather than at the next start.
    pub fn apply_ui(&self, ui: &postio_config::UiConfig) {
        self.list().set_show_actions(ui.show_hover_actions);
        self.list().set_show_hints(ui.show_key_hints);
    }

    /// Reopen where the last session left off.
    ///
    /// Takes `sidebar` directly rather than reading it back with
    /// `self.sidebar()`: this runs from `constructed()` before
    /// `imp().sidebar` is set, so that accessor would panic.
    fn restore(&self, shell: &Shell, sidebar: &Sidebar) {
        let state = WindowState::load();
        self.set_default_size(state.width, state.height);
        self.set_maximized(state.maximized);
        shell.set_divider_positions(state.sidebar_width, state.list_width);
        // The saved *preference*, not a toggle: `set_sidebar_wanted` derives
        // the effective state from it and the current mode, so a window that
        // opens narrow shows no sidebar and still remembers the answer for
        // when it grows (ADR 0024).
        shell.set_sidebar_wanted(state.sidebar_visible);

        // Which folders are closed (#324). A save on every toggle rather
        // than batched with the rest of the window's state: it is cheap,
        // frequent enough that batching would mean losing it more often on
        // a crash, and simple enough not to need `save_state`'s own timing.
        sidebar.set_collapsed(crate::state::SidebarState::load().collapsed_folders);
        sidebar.connect_collapsed_changed(glib::clone!(
            #[weak]
            sidebar,
            move || {
                let state = crate::state::SidebarState {
                    collapsed_folders: sidebar.collapsed(),
                };
                if let Err(error) = state.save() {
                    tracing::warn!(%error, "cannot save which folders are collapsed");
                }
            }
        ));
    }

    /// The state this window would persist, without persisting it.
    ///
    /// Split from [`save_state`](Self::save_state) so that *what* gets saved
    /// is assertable on its own (#852). The write goes to
    /// `WindowState::path()`, the real user state file, which a test may not
    /// touch — and the obvious way round that, overriding `XDG_STATE_HOME`,
    /// would leak into every other case sharing `gtk_suite`'s process.
    ///
    /// `None` before the shell is built, which is the same condition
    /// `save_state` has always returned early on.
    pub fn window_state(&self) -> Option<WindowState> {
        let shell = self.imp().shell.get()?;
        let (sidebar_width, list_width) = shell.divider_positions();
        Some(WindowState {
            width: self.default_width(),
            height: self.default_height(),
            maximized: self.is_maximized(),
            sidebar_width,
            list_width,
            // What the user asked for, never what this window's width could
            // afford. Saving the effective flag meant quitting on a narrow
            // window recorded "no sidebar" as a preference, and nothing at a
            // wider size ever put it back (#825).
            sidebar_visible: shell.sidebar_wanted(),
        })
    }

    /// Write the geometry and the divider positions back out.
    ///
    /// Best-effort: a state file that cannot be written is worth one line on
    /// stderr and nothing more.
    pub fn save_state(&self) {
        let Some(state) = self.window_state() else {
            return;
        };
        if let Err(error) = state.save() {
            // Losing a divider position is a shrug; saying nothing about why
            // it keeps happening is not.
            tracing::warn!(%error, "cannot save the window state");
        }
    }
}

/// Destroy every window that still exists, for a test binary's teardown.
///
/// A `GtkWindow` joins the toplevel list when it is **constructed**, not when
/// it is presented, and leaves it on `destroy()`. So a test that builds a
/// window and drops the handle leaves it alive — and with it its `Reader`,
/// its `WebContext`, and the WebProcess that `WebContext` *is*. At `exit()`
/// the UI process tears those connections down underneath processes that are
/// still running, WebKit says so once per live view, and the binary
/// segfaults after every test in it has passed (#794).
///
/// Public because the fix has to reach binaries that have no harness to hang
/// it on: `gtk_suite` and `app_suite` sweep after every case, but
/// `backend_choice`, `e2e` and their neighbours are one test each with
/// nothing between them and `exit()`.
///
/// Does nothing if GTK was never initialized — a test that skipped for want
/// of a display has no windows, and `toplevels()` panics rather than
/// answering in that state.
pub fn close_all_windows() {
    if !gtk::is_initialized() {
        return;
    }
    let toplevels = gtk::Window::toplevels();
    let windows: Vec<gtk::Window> = (0..toplevels.n_items())
        .filter_map(|item| toplevels.item(item))
        .filter_map(|object| object.downcast::<gtk::Window>().ok())
        .collect();
    for window in windows {
        window.destroy();
    }
    while glib::MainContext::default().iteration(false) {}
}

impl Default for Window {
    fn default() -> Self {
        glib::Object::new()
    }
}
