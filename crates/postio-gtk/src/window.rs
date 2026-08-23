//! The application window: an `AdwApplicationWindow` wearing the generated
//! design tokens.
//!
//! The canvas' PLATE direction keeps *real* Adwaita window chrome — a genuine
//! `AdwHeaderBar`, the compositor's own controls — so Postio reads as a GNOME
//! application rather than as a canvas drawn inside a bare frame. The Industry
//! identity lives in the type, the steel accent and the hairlines inside that
//! chrome, not in a replacement for it.
//!
//! The window owns three things the panes below it should not have to: the
//! header bar, the breakpoints that decide how many panes fit, and the state
//! that has to survive a restart.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::shell::Shell;
use crate::sidebar::Sidebar;
use crate::state::WindowState;
use crate::{header, style};

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

        let layout = adw::ToolbarView::new();
        layout.add_top_bar(&header.bar);
        layout.set_content(Some(&shell));
        self.set_content(Some(&layout));

        // Breakpoints only fire once the window has a size, so the restored
        // state goes on first and the breakpoints correct it if it does not
        // fit.
        self.restore(&shell);
        shell.install_breakpoints(self);
        header.sidebar_toggle.set_active(shell.sidebar_visible());

        let _ = self.imp().shell.set(shell);
        let _ = self.imp().sidebar.set(sidebar);
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
