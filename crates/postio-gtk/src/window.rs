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
use crate::feed::{Feeds, Folders, MailboxSource, MessageSource};
use crate::finder::{Finder, Mode};
use crate::keymap::{self, KeyContext, Outcome, Resolver};
use crate::list_state::ListStateView;
use crate::list_view::MessageListView;
use crate::settings::SettingsPanel;
use crate::shell::Shell;
use crate::sidebar::{Sidebar, SyncStatus};
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

/// The default size, from canvas 1b: a 1120px board over a 52px header bar.
///
/// Wide enough that the three-pane layout is what a first run actually looks
/// like — a mail client that opens into two panes has already lost the
/// argument about what it is.
pub const DEFAULT_SIZE: (i32, i32) = (1120, 700);

/// How much of a thread a drill-in reads.
///
/// One request rather than a paged feed: a thread is a conversation, and the
/// column already holds every message it is given in memory to sort and
/// filter them. `benches/thread_drill.rs` measures the drill-in against a
/// 200-message thread, which is the size this is chosen to clear comfortably;
/// a conversation past it is pathological rather than long, and the header's
/// `n of m` says so honestly.
const THREAD_PAGE: u32 = 500;

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Default)]
    pub struct Window {
        pub shell: OnceCell<Shell>,
        pub sidebar: OnceCell<Sidebar>,
        pub list_state: OnceCell<ListStateView>,
        pub list: OnceCell<MessageListView>,
        /// The list and its named states, together — hidden as one thing
        /// while a thread has the column.
        pub list_pane: OnceCell<gtk::Overlay>,
        /// The thread, where the list was. See [`crate::thread`].
        pub thread: OnceCell<crate::thread::ThreadView>,
        /// Where a drill-in reads the whole thread from.
        ///
        /// The message list's own feed owns this too; the window keeps a
        /// handle because a thread is not a page of the list and cannot be
        /// asked for through it. `None` until `install_feeds`, which is the
        /// state a window built for a test of one widget is in — the drill-in
        /// then shows what the list model holds, exactly as it always did.
        pub messages: std::cell::RefCell<Option<std::rc::Rc<dyn MessageSource>>>,
        /// Switch to a mailbox the way picking it in the sidebar does: set
        /// by [`install_feeds`](super::Window::install_feeds), so
        /// [`open_mailbox`](super::Window::open_mailbox) is a no-op before
        /// the window has been fed anything to switch to.
        pub open_mailbox: std::cell::RefCell<Option<OpenMailbox>>,
        /// Where the list was scrolled to when the drill-in hid it.
        pub list_scroll: std::cell::Cell<f64>,
        pub finder: OnceCell<Finder>,
        pub cheatsheet: OnceCell<CheatSheet>,
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
        /// Where the reader resolves `cid:` parts from.
        ///
        /// A slot rather than a constructor argument, so the reader can be
        /// built before storage has been wired and start resolving parts the
        /// moment something supplies a source — the same shape the search
        /// preview uses, and for the same reason.
        pub blobs: std::cell::RefCell<Option<std::rc::Rc<dyn crate::reader::BlobSource>>>,
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
        pub overlay: OnceCell<gtk::Overlay>,
        pub resolver: OnceCell<std::cell::RefCell<Resolver>>,
        /// `None` until `build` sets it; the accessor reads it as `List`.
        pub context: std::cell::Cell<Option<Context>>,
        pub commands: std::cell::RefCell<Vec<CommandHandler>>,
        /// Handlers for whole invocations, which the mouse produces — see
        /// [`Window::connect_action`](super::Window::connect_action).
        pub actions: std::cell::RefCell<Vec<ActionHandler>>,
        /// Handlers for commands registered at runtime — see
        /// [`Window::connect_ext_command`](super::Window::connect_ext_command).
        pub ext_commands: std::cell::RefCell<Vec<ExtCommandHandler>>,
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
    }

    /// Put the parts panel away.
    pub fn close_parts(&self) {
        self.parts().set_visible(false);
    }

    /// The thread column, shown in the list's place while drilled in.
    pub fn thread(&self) -> crate::thread::ThreadView {
        self.imp()
            .thread
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Whether a thread has the list column.
    pub fn thread_open(&self) -> bool {
        self.imp()
            .thread
            .get()
            .is_some_and(|thread| thread.thread().is_some())
    }

    /// Drill into `row`'s thread.
    ///
    /// The messages are the ones the list already holds for that thread — see
    /// [`crate::thread`] for why that is both enough to be useful today and
    /// not always the whole thread.
    ///
    /// Public so a test, and whoever wires a real thread read later, can put
    /// the column up without synthesizing a key event.
    pub fn open_thread(&self, row: &crate::list::Row) {
        let Some(id) = row.thread else { return };
        // What the list already holds, first and synchronously. A drill-in is
        // an ordinary interaction and owes an answer inside the 16ms budget;
        // waiting for a read would make `t` feel like a load. This is the
        // same local-first shape every mutating action here uses.
        self.show_thread(
            id,
            row.subject.as_deref(),
            self.thread_rows(id),
            row.thread_count,
        );

        // Then the rest of it. A thread routinely spans folders, and the part
        // of it in *this* folder is all the list model has ever been able to
        // offer -- less than that, if the page carrying a message has not been
        // scrolled to. See #44.
        let Some(source) = self.imp().messages.borrow().clone() else {
            return;
        };
        let future = source.fetch(crate::feed::PageRequest {
            scope: crate::feed::FeedScope::Thread(id),
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
                    Ok(page) => window.thread().fill(id, page.rows, page.total),
                    // The column keeps what the list gave it, which is a
                    // subset rather than nothing, and the header goes on
                    // saying `n of m`. Worth a line, not a banner.
                    Err(message) => tracing::debug!(message, "the thread could not be read"),
                }
            }
        ));
    }

    /// Put `thread` in the column, with the messages you name.
    ///
    /// The half of [`Window::open_thread`] that does not read the list model,
    /// so a test or a render can put the column up without one — and does it
    /// through the same swap the application makes, rather than beside it.
    pub fn show_thread(
        &self,
        thread: postio_model::ids::ThreadId,
        subject: Option<&str>,
        rows: Vec<crate::list::Row>,
        total: u32,
    ) {
        let view = self.thread();
        // Shown *before* it is filled. A `GtkListView` with no allocation
        // cannot know how many rows fit, so it binds far more of the model
        // than a screenful — filling it while hidden was measured at 16ms for
        // a 200-message thread against a 16ms interaction budget, and at
        // under 1ms once the viewport had a height to answer with.
        self.imp().list_scroll.set(self.list().scroll_offset());
        if let Some(pane) = self.imp().list_pane.get() {
            pane.set_visible(false);
        }
        view.set_visible(true);
        view.open(
            thread,
            subject,
            rows,
            total,
            Some(&self.list().mailbox_name()),
        );

        // The swap is a `set_visible` and nothing else: no reparenting, no
        // revealer, no transition. The list keeps its model, its cursor and
        // its selection because nothing here touches them, which is most of
        // what makes `Esc` restore the position exactly rather than
        // approximately. Hiding the *overlay* rather than the list takes the
        // named states with it — an "Offline, reading local mail" panel that
        // stayed up over the thread would be answering a question nobody
        // asked.
        //
        // The scroll offset is the one thing that does not survive by itself,
        // and it is worth being precise about why: nothing here moves it, but
        // giving the list the keyboard back on the way out scrolls the cursor
        // row into view, and "into view" is not the pixel offset the user
        // left. Measured at two rows of drift on a 200-message list.
        view.focus_rows();
        self.set_context(Context::Thread);
    }

    /// Leave the thread and put the list back.
    pub fn close_thread(&self) {
        if !self.thread_open() {
            return;
        }
        let thread = self.thread();
        thread.set_visible(false);
        thread.close();
        if let Some(pane) = self.imp().list_pane.get() {
            pane.set_visible(true);
        }
        // Focus first, then put the offset back — on a frame tick, not on an
        // idle.
        //
        // Grabbing the keyboard scrolls the cursor row into view, and "into
        // view" is not the pixel offset the user left: two rows of drift on a
        // 200-message list, 84px of it here.
        //
        // So the offset has to go back *after* that scroll. `idle_add` is the
        // obvious way and it is not ordered against the frame clock, which
        // drives the layout pass that performs the scroll — so it restored
        // correctly about half the time and left exactly those two rows the
        // other half (`postio-1ff`: 6 of 12 runs on an idle box, always the
        // same 84px). Restoring *before* the grab does not work either: the
        // scroll-into-view happens regardless of whether the row is already
        // visible, so a synchronous restore is simply overwritten — that
        // variant failed 20 of 20.
        //
        // A tick callback is ordered: it runs on the frame clock, so waiting
        // one full frame puts this strictly after the layout pass that did
        // the scrolling. The first tick can be the one the scroll happens in,
        // which is why it takes two.
        let offset = self.imp().list_scroll.get();
        let list = self.list();
        list.grab_focus();
        let ticks = std::cell::Cell::new(0u8);
        list.clone().add_tick_callback(move |list, _| {
            ticks.set(ticks.get() + 1);
            if ticks.get() < 2 {
                return glib::ControlFlow::Continue;
            }
            list.set_scroll_offset(offset);
            glib::ControlFlow::Break
        });
        self.set_context(Context::List);
    }

    /// The first row the list is holding for `thread`, for its subject and
    /// its thread count.
    fn row_in_thread(&self, thread: postio_model::ids::ThreadId) -> Option<crate::list::Row> {
        self.thread_rows(thread).into_iter().next()
    }

    /// The rows the list is holding for `thread`.
    ///
    /// Read off the model rather than asked for, which is what lets the
    /// drill-in work without a thread query behind it. The model is windowed,
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
        composer.connect_opened({
            let window = self.clone();
            move || window.sync_reading_pane()
        });
        composer.connect_closed({
            let window = self.clone();
            move |_| window.sync_reading_pane()
        });
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
    pub fn reader(&self) -> crate::reader::Reader {
        if let Some(reader) = self.imp().reader.get() {
            return reader.clone();
        }
        // Read through the slot on every request rather than captured, so a
        // source wired after the reader was built still resolves parts.
        let source = {
            let window = self.clone();
            move |content_id: &str| {
                let blobs = window.imp().blobs.borrow();
                blobs.as_ref().and_then(|blobs| blobs.resolve(content_id))
            }
        };
        let reader = crate::reader::Reader::new(std::rc::Rc::new(source));
        let widget = reader.widget();
        widget.set_vexpand(true);
        // Nothing to read yet. The pane shows its empty state until a message
        // arrives, rather than an empty white rectangle pretending to be one.
        widget.set_visible(false);
        self.shell().reader().append(&widget);
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

    /// Show a message in the reading pane.
    ///
    /// `sender` is the allow-list key: remote images stay blocked until this
    /// sender is allowed, which is [`crate::reader::Reader`]'s own rule and
    /// is not something this can bypass.
    pub fn show_message(&self, body: &postio_model::MessageBody, sender: Option<&str>) {
        let reader = self.reader();
        reader.render(body, sender);
        self.imp().reading.set(true);
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
        self.imp().reading.set(true);
        self.sync_reading_pane();
    }

    /// Empty the reading pane — the folder changed, or the message went away.
    pub fn clear_reader(&self) {
        if let Some(reader) = self.imp().reader.get() {
            reader.clear();
        }
        self.imp().reading.set(false);
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
        if let Some(reader) = self.imp().reader.get() {
            reader.widget().set_visible(self.reading());
        }
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
                    // on the way to the header above the rows.
                    list.set_mailbox(
                        &crate::sidebar::display_name(&mailbox),
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
            move |_| {
                if feed.mailbox().is_some() {
                    return;
                }
                if let Some(id) = sidebar.selected().or_else(|| folders.default_mailbox()) {
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
            move |status| window.refresh_list_state(status)
        ));
        list.model().connect_items_changed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            #[strong]
            folders,
            move |_, _, _, _| window.refresh_list_state(&folders.status())
        ));

        folders.open(account, address);
        *self.imp().messages.borrow_mut() = Some(messages);
        Feeds {
            messages: feed,
            folders,
        }
    }

    /// Re-derive the list pane's named state from `status`.
    ///
    /// `stored` and `queued` are what the local store still holds and what is
    /// waiting to reach the server. Neither has a cheap accessor on this side
    /// of the crate boundary yet — `postio-storage`'s operation queue has no
    /// count — so they are reported as what the pane can actually see, and
    /// `postio-qhz` will widen them when the counts exist.
    fn refresh_list_state(&self, status: &SyncStatus) {
        let rows = self.list().model().n_items() as u64;
        self.list_state().set_status(status.clone(), rows, rows, 0);
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

        // Canvas 3a: `t` turns this column into the thread. Built alongside
        // the list rather than lazily, because the swap has to be a
        // `set_visible` and nothing else — a pane that had to be constructed
        // on the way in could not be instant, and the motion budget says pane
        // switches do not animate at all.
        let thread = crate::thread::ThreadView::new();
        thread.set_vexpand(true);
        shell.list().append(&thread);
        thread.connect_back(glib::clone!(
            #[weak(rename_to = window)]
            self,
            // The button runs the registry's own `Back`, so leaving a thread
            // is one path whether it was the mouse or `Esc` that asked.
            move || window.run(CommandId::Back)
        ));
        let _ = self.imp().thread.set(thread);
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
        let parts = crate::parts::PartsPanel::new();
        parts.connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            // Through the registry's `Back`, so `Esc` in here and `Esc`
            // anywhere else are one path.
            move || window.run(CommandId::Back)
        ));
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
        self.restore(&shell);
        shell.install_breakpoints(self);
        header.sidebar_toggle.set_active(shell.sidebar_visible());

        let _ = self.imp().shell.set(shell);
        let _ = self.imp().sidebar.set(sidebar);
        let _ = self.imp().list_state.set(list_state);
        let _ = self.imp().list.set(list_view);
        let _ = self.imp().finder.set(finder);
        let _ = self.imp().cheatsheet.set(cheatsheet);
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
        let _ = self.imp().resolver.set(std::cell::RefCell::new(resolver));
        // The registry's own bindings, so the box and the cheat sheet print
        // keys from the first frame rather than from whenever `config.toml`
        // gets around to being read. `apply_keymap` replaces them if it
        // says something different.
        self.finder().set_keymap(keymap.clone());
        self.cheatsheet().set_keymap(keymap);

        let finder = self.finder();
        finder.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| {
                window.close_finder();
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
            move |_| window.close_finder()
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
        let typing = gtk::prelude::GtkWindowExt::focus(source.as_ref())
            .is_some_and(|focus| focus.is::<gtk::Text>() || focus.is::<gtk::TextView>());
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
        match id {
            ActionId::Builtin(id) => self.run(id),
            ActionId::Ext(id) => {
                for handler in self.imp().ext_commands.borrow().iter() {
                    handler(id);
                }
            }
        }
    }

    /// Swaps the list column for the thread, or back, and says what the
    /// application should be told if it did.
    ///
    /// `None` means this was not a drill-in and the caller's own command
    /// stands. Unlike `handled_here` this acts *and* lets the command go out:
    /// the panes swap here, and `AppState` has to hear about it or its back
    /// stack and its keyboard context drift out of step with what is on
    /// screen.
    fn follow_drill_in(&self, command: &postio_core::Command) -> Option<postio_core::Command> {
        match command {
            postio_core::Command::Thread { thread } if !self.thread_open() => {
                // A keystroke names only the verb, so it means the row the
                // cursor is on. An invocation that names a thread means that
                // one, wherever the cursor happens to be.
                let row = match thread {
                    Some(id) => self.row_in_thread(*id)?,
                    None => self.list().cursor_row()?,
                };
                // A message threading has not placed yet has no thread to
                // drill into. Leave the key alone rather than opening a column
                // that would have to explain itself.
                let thread = row.thread?;
                self.open_thread(&row);
                Some(postio_core::Command::Thread {
                    thread: Some(thread),
                })
            }
            postio_core::Command::Back if self.thread_open() => {
                self.close_thread();
                None
            }
            _ => None,
        }
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
            CommandId::Settings => self.toggle_settings(),
            CommandId::Search => self.open_finder(Mode::Search),
            // One `Esc` closes one overlay, nearest first — and a selection
            // is the nearest thing of all once every overlay is shut. It is
            // also the only way out of one that does not require picking a
            // row, which matters most when the selection is a predicate.
            // Nearest first. The parts panel is the innermost thing `Esc`
            // could mean while it is up.
            CommandId::Back if self.parts().is_visible() => self.close_parts(),
            CommandId::Back if self.cheatsheet().is_visible() => self.close_cheatsheet(),
            CommandId::Back if self.finder().is_open() => self.close_finder(),
            CommandId::Back if self.settings().is_visible() => self.close_settings(),
            // Nearer than a selection made before the keyboard went to the
            // folders: `Esc` in the sidebar means "back to the messages".
            CommandId::Back if self.context() == Context::Sidebar => self.leave_sidebar(),
            // Not while a thread has the column: `Esc` there means "back to
            // the list", which is nearer than a selection made before the
            // drill-in. It falls through to `follow_drill_in`, which needs to
            // tell the application as well as move the panes.
            CommandId::Back if !self.thread_open() && !self.list().selection().is_empty() => {
                self.list().clear_selection()
            }

            // Where the keyboard is, and what an action would hit. Two
            // different things, moved by two different sets of keys — see
            // `crate::selection`.
            // `j`/`k` mean the same verb in both columns; which column they
            // move is a fact about what is on screen, not a second binding.
            CommandId::NextMessage if self.thread_open() => self.thread().next_row(),
            CommandId::PrevMessage if self.thread_open() => self.thread().prev_row(),
            CommandId::FirstMessage if self.thread_open() => self.thread().first_row(),
            CommandId::LastMessage if self.thread_open() => self.thread().last_row(),
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
            CommandId::NextFolder => {
                self.sidebar().step(1);
            }
            CommandId::PrevFolder => {
                self.sidebar().step(-1);
            }
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

    /// Give the keyboard back to whatever had it before the folders.
    fn leave_sidebar(&self) {
        let previous = self.imp().before_sidebar.take().unwrap_or(Context::List);
        self.set_context(previous);
        if self.thread_open() {
            self.thread().grab_focus();
        } else {
            self.list().grab_focus();
        }
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
    }

    /// Called with every command a key press resolves to.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().commands.borrow_mut().push(Box::new(handler));
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
        // A command the window answers itself — closing an overlay, moving
        // the cursor — means the same thing however it was invoked, and stops
        // here either way.
        if self.handled_here(command.id()) {
            return;
        }
        let command = self.follow_drill_in(&command).unwrap_or(command);
        self.deliver(command);
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
        self.shell().grab_focus();
    }

    /// The `?` overlay.
    pub fn cheatsheet(&self) -> CheatSheet {
        self.imp()
            .cheatsheet
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
        // Two overlays at once is one too many.
        self.close_finder();
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
        // Only one overlay at a time.
        self.close_finder();
        self.close_cheatsheet();
        self.settings().set_visible(true);
        self.settings().grab_focus();
    }

    /// Hides the settings panel and gives the keyboard back to the workspace.
    pub fn close_settings(&self) {
        self.settings().set_visible(false);
        self.shell().grab_focus();
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
        }
        self.finder().set_keymap(keymap.clone());
        self.cheatsheet().set_keymap(keymap);
    }

    /// Apply the `[ui]` block: what the list shows, and how much of it.
    ///
    /// Live — the config watcher calls this on every save that changes the
    /// section, so turning hover actions off in `config.toml` takes them off
    /// the rows already on screen rather than at the next start.
    pub fn apply_ui(&self, ui: &postio_config::UiConfig) {
        self.list().set_show_actions(ui.show_hover_actions);
    }

    /// Reopen where the last session left off.
    fn restore(&self, shell: &Shell) {
        let state = WindowState::load();
        self.set_default_size(state.width, state.height);
        self.set_maximized(state.maximized);
        shell.set_divider_positions(state.sidebar_width, state.list_width);
        shell.set_sidebar_visible(state.sidebar_visible);
    }

    /// Write the geometry and the divider positions back out.
    ///
    /// Best-effort: a state file that cannot be written is worth one line on
    /// stderr and nothing more.
    pub fn save_state(&self) {
        let Some(shell) = self.imp().shell.get() else {
            return;
        };
        let (sidebar_width, list_width) = shell.divider_positions();
        let state = WindowState {
            width: self.default_width(),
            height: self.default_height(),
            maximized: self.is_maximized(),
            sidebar_width,
            list_width,
            sidebar_visible: shell.sidebar_visible(),
        };
        if let Err(error) = state.save() {
            // Losing a divider position is a shrug; saying nothing about why
            // it keeps happening is not.
            tracing::warn!(%error, "cannot save the window state");
        }
    }
}

impl Default for Window {
    fn default() -> Self {
        glib::Object::new()
    }
}
