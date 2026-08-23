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

use postio_core::{CommandId, Context};

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

/// The default size, from canvas 1b: a 1120px board over a 52px header bar.
///
/// Wide enough that the three-pane layout is what a first run actually looks
/// like — a mail client that opens into two panes has already lost the
/// argument about what it is.
pub const DEFAULT_SIZE: (i32, i32) = (1120, 700);

mod imp {
    use std::cell::OnceCell;

    use super::*;

    #[derive(Default)]
    pub struct Window {
        pub shell: OnceCell<Shell>,
        pub sidebar: OnceCell<Sidebar>,
        pub list_state: OnceCell<ListStateView>,
        pub list: OnceCell<MessageListView>,
        pub finder: OnceCell<Finder>,
        pub cheatsheet: OnceCell<CheatSheet>,
        /// Installed lazily, on first [`Window::composer`] — nothing before
        /// that call needs it, and the composition root is the one place
        /// that both installs and wires it.
        pub composer: OnceCell<crate::composer::Composer>,

        pub settings: OnceCell<SettingsPanel>,
        /// The pane that had the keyboard when the box opened.
        pub before_finder: std::cell::Cell<Option<(Context, crate::shell::Pane)>>,
        pub overlay: OnceCell<gtk::Overlay>,
        pub resolver: OnceCell<std::cell::RefCell<Resolver>>,
        /// `None` until `build` sets it; the accessor reads it as `List`.
        pub context: std::cell::Cell<Option<Context>>,
        pub commands: std::cell::RefCell<Vec<CommandHandler>>,
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

    /// The composer, installing it into the reading pane the first time
    /// anyone asks.
    ///
    /// Lazy rather than built alongside the other panes in `constructed`: a
    /// window used only for a test of, say, the sidebar has no reason to pay
    /// for a composer nobody opens. Whoever wires storage to it — the
    /// composition root — is the one place that needs this at all.
    pub fn composer(&self) -> crate::composer::Composer {
        self.imp()
            .composer
            .get_or_init(|| crate::composer::install(self))
            .clone()
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
        let feed = list.feed(messages);
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
                feed.open(id);
            })
        };

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

        // The bulk bar runs the same commands the keyboard does, through the
        // same path: a button that acted directly would be a second
        // implementation of a verb the registry already owns.
        list_view.connect_command(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| window.run(id)
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
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&shell));
        overlay.add_overlay(&finder);
        overlay.add_overlay(&cheatsheet);
        overlay.add_overlay(&settings);

        let layout = adw::ToolbarView::new();
        layout.add_top_bar(&header.bar);
        layout.set_content(Some(&overlay));
        self.set_content(Some(&layout));

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
                window.dispatch(id);
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
        let Some(chord) = keymap::Chord::from_key_event(key, state) else {
            return glib::Propagation::Proceed;
        };
        let Some(resolver) = self.imp().resolver.get() else {
            return glib::Propagation::Proceed;
        };

        let context = self.key_context();
        let typing = self.is_typing();
        let outcome =
            resolver
                .borrow_mut()
                .press(&chord, context, typing, std::time::Instant::now());

        match outcome {
            Outcome::Command(id) => match id.parse::<CommandId>() {
                Ok(id) => {
                    self.run(id);
                    glib::Propagation::Stop
                }
                // A binding for a command this build does not know: leave the
                // key alone rather than swallowing it.
                Err(_) => glib::Propagation::Proceed,
            },
            // A half-typed sequence is consumed so its first chord does not also
            // reach the widget underneath.
            Outcome::Pending(_) => glib::Propagation::Stop,
            Outcome::Unhandled => glib::Propagation::Proceed,
        }
    }

    /// Acts on the commands that are about the window, and passes on the rest.
    fn run(&self, id: CommandId) {
        match id {
            CommandId::CommandPalette => self.open_finder(Mode::Command),
            CommandId::CheatSheet => self.toggle_cheatsheet(),
            CommandId::Settings => self.toggle_settings(),
            CommandId::Search => self.open_finder(Mode::Search),
            // One `Esc` closes one overlay, nearest first — and a selection
            // is the nearest thing of all once every overlay is shut. It is
            // also the only way out of one that does not require picking a
            // row, which matters most when the selection is a predicate.
            CommandId::Back if self.cheatsheet().is_visible() => self.close_cheatsheet(),
            CommandId::Back if self.finder().is_open() => self.close_finder(),
            CommandId::Back if self.settings().is_visible() => self.close_settings(),
            CommandId::Back if !self.list().selection().is_empty() => self.list().clear_selection(),

            // Where the keyboard is, and what an action would hit. Two
            // different things, moved by two different sets of keys — see
            // `crate::selection`.
            CommandId::NextMessage => self.list().next_row(),
            CommandId::PrevMessage => self.list().prev_row(),
            CommandId::FirstMessage => self.list().first_row(),
            CommandId::LastMessage => self.list().last_row(),
            CommandId::ToggleSelection => self.list().toggle_cursor_row(),
            CommandId::ExtendSelectionDown => self.list().extend_down(),
            CommandId::ExtendSelectionUp => self.list().extend_up(),
            CommandId::SelectAll => self.list().select_all(),
            _ => self.dispatch(id),
        }
    }

    fn dispatch(&self, id: CommandId) {
        for handler in self.imp().commands.borrow().iter() {
            handler(id);
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
        // The box owns the keyboard while it is open, and which of its two
        // contexts depends on the mode: `Enter` runs a command in one and
        // searches in the other.
        match self.finder().context() {
            Some(context) => KeyContext::from(context),
            None => KeyContext::from(self.context()),
        }
    }
}

fn report(problems: &[String]) {
    for problem in problems {
        eprintln!("postio: {problem}");
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

    /// Opens the box in `mode`, remembering what to come back to.
    ///
    /// No animation and no dialog: the field is typeable the instant it has
    /// the keyboard, which is what the canvas means by search being
    /// navigation rather than a mode you enter.
    pub fn open_finder(&self, mode: Mode) {
        let finder = self.finder();
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
            eprintln!("postio: cannot save the window state: {error}");
        }
    }
}

impl Default for Window {
    fn default() -> Self {
        glib::Object::new()
    }
}
