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
use crate::keymap::{self, KeyContext, Outcome, Resolver};
use crate::palette::Palette;
use crate::search::SearchBar;
use crate::settings::SettingsPanel;
use crate::shell::Shell;
use crate::sidebar::Sidebar;
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
        pub palette: OnceCell<Palette>,
        pub cheatsheet: OnceCell<CheatSheet>,
        pub search: OnceCell<SearchBar>,
        pub settings: OnceCell<SettingsPanel>,
        /// The pane that had the keyboard when search opened.
        pub before_search: std::cell::Cell<Option<(Context, crate::shell::Pane)>>,
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

        // The palette floats over the workspace rather than replacing it: the
        // canvas shows the panes still visible behind it, and a palette that
        // blanked the window would lose the context the user is choosing in.
        let palette = Palette::new();
        palette.set_visible(false);
        let cheatsheet = CheatSheet::new();
        cheatsheet.set_visible(false);
        let search = SearchBar::new();
        search.set_visible(false);
        search.set_valign(gtk::Align::Start);
        let settings = SettingsPanel::new();
        settings.set_visible(false);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&shell));
        overlay.add_overlay(&palette);
        overlay.add_overlay(&search);
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
        let _ = self.imp().palette.set(palette);
        let _ = self.imp().cheatsheet.set(cheatsheet);
        let _ = self.imp().search.set(search);
        let _ = self.imp().settings.set(settings);
        let _ = self.imp().overlay.set(overlay);
        self.imp().context.set(Some(Context::List));

        self.install_keyboard();
    }

    /// Builds the resolver from the registry defaults and starts listening.
    fn install_keyboard(&self) {
        let (resolver, problems) =
            Resolver::from_commands(&postio_core::Keymap::resolve(&Default::default()));
        report(&problems);
        let _ = self.imp().resolver.set(std::cell::RefCell::new(resolver));

        let palette = self.palette();
        palette.connect_activated(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move |id| {
                window.close_palette();
                window.dispatch(id);
            }
        ));
        palette.connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close_palette()
        ));

        self.cheatsheet().connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close_cheatsheet()
        ));

        self.search().connect_dismissed(glib::clone!(
            #[weak(rename_to = window)]
            self,
            move || window.close_search()
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
            CommandId::CommandPalette => self.open_palette(),
            CommandId::CheatSheet => self.toggle_cheatsheet(),
            CommandId::Search => self.open_search(),
            // One `Esc` closes one overlay, nearest first.
            CommandId::Back if self.cheatsheet().is_visible() => self.close_cheatsheet(),
            CommandId::Back if self.palette().is_visible() => self.close_palette(),
            CommandId::Back if self.search().is_visible() => self.close_search(),
            CommandId::Back if self.settings().is_visible() => self.close_settings(),
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
        if self.palette().is_visible() {
            return KeyContext::Palette;
        }
        if self.search().is_visible() {
            return KeyContext::Search;
        }
        KeyContext::from(self.context())
    }
}

fn report(problems: &[String]) {
    for problem in problems {
        eprintln!("postio: {problem}");
    }
}

impl Window {
    /// The `Ctrl+K` overlay.
    pub fn palette(&self) -> Palette {
        self.imp()
            .palette
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
        self.palette().set_context(context);
    }

    /// Called with every command a key press resolves to.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().commands.borrow_mut().push(Box::new(handler));
    }

    /// Opens the palette over the workspace, filtered to the current context.
    pub fn open_palette(&self) {
        let palette = self.palette();
        palette.set_context(self.context());
        palette.set_visible(true);
        palette.focus_search();
    }

    /// Closes the palette and gives the keyboard back to the workspace.
    pub fn close_palette(&self) {
        let palette = self.palette();
        palette.set_visible(false);
        if let Some(resolver) = self.imp().resolver.get() {
            resolver.borrow_mut().clear_pending();
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
        self.close_palette();
        let sheet = self.cheatsheet();
        sheet.set_visible(true);
        sheet.grab_focus();
    }

    /// Hides the cheat sheet.
    pub fn close_cheatsheet(&self) {
        self.cheatsheet().set_visible(false);
        self.shell().grab_focus();
    }

    /// The `/` query bar.
    pub fn search(&self) -> SearchBar {
        self.imp()
            .search
            .get()
            .expect("built in constructed")
            .clone()
    }

    /// Opens the query bar over the list, remembering what to come back to.
    ///
    /// No animation and no dialog: the bar is typeable the instant it appears,
    /// which is what the canvas means by search being navigation rather than a
    /// mode you enter.
    pub fn open_search(&self) {
        if self.search().is_visible() {
            self.search().focus_entry();
            return;
        }
        self.close_palette();
        self.close_cheatsheet();

        // Remembered before anything moves, so `Esc` puts the keyboard back
        // where the user left it rather than wherever the bar happened to
        // leave it.
        self.imp()
            .before_search
            .set(Some((self.context(), self.shell().focused_pane())));

        let bar = self.search();
        bar.set_visible(true);
        self.set_context(Context::Search);
        bar.focus_entry();
    }

    /// Closes the query bar and restores the view it opened over.
    pub fn close_search(&self) {
        let bar = self.search();
        if !bar.is_visible() {
            return;
        }
        bar.set_visible(false);
        if let Some((context, pane)) = self.imp().before_search.take() {
            self.set_context(context);
            self.shell().set_focused_pane(pane);
        }
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
        self.close_palette();
        self.close_cheatsheet();
        self.close_search();
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
        self.palette().set_keymap(keymap.clone());
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
