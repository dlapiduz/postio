//! The application window: an `AdwApplicationWindow` wearing the generated
//! design tokens.
//!
//! The canvas' PLATE direction keeps *real* Adwaita window chrome — a genuine
//! `AdwHeaderBar`, the compositor's own controls — so Postio reads as a GNOME
//! application rather than as a canvas drawn inside a bare frame. The Industry
//! identity lives in the type, the steel accent and the hairlines inside that
//! chrome, not in a replacement for it.
//!
//! What is here today is the shell: chrome, size and scheme. The three panes
//! that fill it are the next bead.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

use crate::style;

/// The default size, from canvas 1b: a 1120px board over a 52px header bar.
///
/// Wide enough that the three-pane layout is what a first run actually looks
/// like — a mail client that opens into two panes has already lost the
/// argument about what it is.
pub const DEFAULT_SIZE: (i32, i32) = (1120, 700);

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Window;

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

    /// A window with no application, for tests and for previewing a widget.
    fn build(&self) {
        self.set_title(Some("Postio"));
        self.set_default_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1);
        self.add_css_class("postio-window");

        // Every window carries its own scheme classes: `tokens.css` keys its
        // dark and high-contrast blocks off `:root`, which in GTK is the root
        // *widget*, so the variables have to land here rather than on the
        // application. See `crate::style`.
        style::track(self);

        let header = adw::HeaderBar::new();

        // The shell that fills this is the next bead; the empty content area
        // is deliberately the plain ground rather than a placeholder that
        // would have to be designed and then deleted.
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        content.add_css_class("postio-shell");
        content.set_hexpand(true);
        content.set_vexpand(true);

        let layout = adw::ToolbarView::new();
        layout.add_top_bar(&header);
        layout.set_content(Some(&content));
        self.set_content(Some(&layout));
    }
}

impl Default for Window {
    fn default() -> Self {
        glib::Object::new()
    }
}
