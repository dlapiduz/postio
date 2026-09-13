//! The `?` cheat sheet.
//!
//! # Generated, so it cannot be wrong
//!
//! A hand-written key reference is out of date by the next release — that is
//! the whole reason [`postio_core::registry`] exists (docs/PRODUCT.md §8: one table,
//! every surface). This sheet is rendered from that table and from the live
//! [`Keymap`], so rebinding a key in `config.toml` changes what the overlay
//! says without anybody editing a list.
//!
//! # Two halves
//!
//! [`sections`] decides what the sheet contains and is a pure function with
//! no display — it lives in [`postio_ui::cheatsheet`] now, where a second
//! frontend reaches it, and is re-exported here so nothing changed for this
//! crate. [`CheatSheet`] is the widget around it.
//!
//! # It answers "what can I do *now*"
//!
//! The sheet lists what is reachable where the reader is standing — their
//! context, and the scope on screen — rather than the whole vocabulary. A key
//! that would do nothing here is not a key worth teaching, and `?` is pressed
//! by somebody who is stuck rather than by somebody browsing. The complete
//! reference is `docs/keybindings.md`, which is generated from the same table
//! and is where "what does `m` do" gets answered whatever is on screen.
//!
//! # Grouping
//!
//! Two headings, because after filtering there are only two useful answers:
//! **Everywhere** for commands reachable in every context, and the reader's
//! own surface for the rest. Filing by a command's *first* context — which is
//! what this did while it listed everything — would print "Reading" over keys
//! the reader can press in *Parts*, which is exactly the sort of almost-right
//! that a reference must not do.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::{Availability, Context, Keymap, Scope};

pub use postio_ui::cheatsheet::{Row, Section, sections, spoken};

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct CheatSheet {
        pub columns: gtk::Box,
        pub keymap: RefCell<Keymap>,
        /// Where the reader is standing, and what is on screen. The sheet
        /// lists what is reachable from there rather than the whole
        /// vocabulary (#182).
        pub context: RefCell<Context>,
        pub availability: RefCell<Availability>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
    }

    impl Default for CheatSheet {
        fn default() -> Self {
            Self {
                columns: gtk::Box::new(gtk::Orientation::Horizontal, 32),
                keymap: RefCell::new(Keymap::default()),
                context: RefCell::new(Context::List),
                // A sheet built before anything has fed the window is a
                // sheet over a window with no store, and it lists what that
                // window can actually do (#1114).
                availability: RefCell::new(Availability {
                    scope: Scope::default(),
                    store_open: false,
                }),
                dismissed: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for CheatSheet {
        const NAME: &'static str = "PostioCheatSheet";
        type Type = super::CheatSheet;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for CheatSheet {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for CheatSheet {}
    impl BinImpl for CheatSheet {}
}

glib::wrapper! {
    /// The `?` overlay: every key, generated from the registry.
    pub struct CheatSheet(ObjectSubclass<imp::CheatSheet>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for CheatSheet {
    fn default() -> Self {
        glib::Object::new()
    }
}

/// How many heading groups go in one column before a new one starts.
///
/// The sheet is read at a glance, so it goes across before it goes down.
const SECTIONS_PER_COLUMN: usize = 3;

impl CheatSheet {
    /// A sheet over the registry, with no bindings until one is given.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bindings to print.
    ///
    /// Call it whenever `[keys]` changes; the sheet is rebuilt from it, which
    /// is the whole of "rebinding a key changes the cheat sheet".
    pub fn set_keymap(&self, keymap: Keymap) {
        *self.imp().keymap.borrow_mut() = keymap;
        self.rebuild();
    }

    /// Where the reader is standing. The sheet answers from there.
    pub fn set_context(&self, context: Context) {
        *self.imp().context.borrow_mut() = context;
        self.rebuild();
    }

    /// What the window can currently do: its scope, and whether the store
    /// behind it is open — see [`Availability`].
    pub fn set_availability(&self, state: Availability) {
        *self.imp().availability.borrow_mut() = state;
        self.rebuild();
    }

    /// What the sheet currently lists.
    pub fn sections(&self) -> Vec<Section> {
        sections(
            &self.imp().keymap.borrow(),
            *self.imp().context.borrow(),
            *self.imp().availability.borrow(),
        )
    }

    /// Called when the user presses `Escape` or `?` again.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.imp().dismissed.borrow_mut().push(Box::new(handler));
    }

    /// Dismisses the sheet, as `Escape` does.
    pub fn dismiss(&self) {
        for handler in self.imp().dismissed.borrow().iter() {
            handler();
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-cheatsheet");
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Center);

        // A dialog to a screen reader, because that is what it behaves like:
        // it takes the keyboard and `Escape` closes it.
        self.set_accessible_role(gtk::AccessibleRole::Dialog);
        self.update_property(&[gtk::accessible::Property::Label("Keyboard shortcuts")]);

        let heading = gtk::Label::new(Some("Keyboard shortcuts"));
        heading.set_xalign(0.0);
        heading.add_css_class("postio-cheatsheet-heading");

        let column = gtk::Box::new(gtk::Orientation::Vertical, 12);
        column.append(&heading);
        column.append(&imp.columns);
        self.set_child(Some(&column));

        // `Escape` closes it. So does `?`, which is what the user pressed to
        // open it — a sheet that a second press of its own key cannot close is
        // one people get stuck in.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = sheet)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if matches!(key, gtk::gdk::Key::Escape | gtk::gdk::Key::question) {
                    sheet.dismiss();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(keys);

        self.rebuild();
    }

    /// Rebuilds the whole sheet.
    ///
    /// Every time, from scratch. It is thirty rows built once per opening, so
    /// there is nothing to gain from an incremental update and a stale row to
    /// lose by it.
    fn rebuild(&self) {
        let imp = self.imp();
        while let Some(child) = imp.columns.first_child() {
            imp.columns.remove(&child);
        }

        let sections = self.sections();
        for group in sections.chunks(SECTIONS_PER_COLUMN) {
            let column = gtk::Box::new(gtk::Orientation::Vertical, 16);
            column.set_valign(gtk::Align::Start);
            for section in group {
                column.append(&section_widget(section));
            }
            imp.columns.append(&column);
        }
    }
}

fn section_widget(section: &Section) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 4);

    let heading = gtk::Label::new(Some(section.title));
    heading.set_xalign(0.0);
    heading.add_css_class("postio-kicker");
    group.append(&heading);

    let grid = gtk::Grid::new();
    grid.set_row_spacing(2);
    grid.set_column_spacing(16);
    for (index, row) in section.rows.iter().enumerate() {
        let line = index as i32;

        let title = gtk::Label::new(Some(row.title));
        title.set_xalign(0.0);
        title.set_hexpand(true);
        title.add_css_class("postio-cheatsheet-title");
        grid.attach(&title, 0, line, 1, 1);

        let key = gtk::Label::new(Some(row.binding.as_deref().unwrap_or("—")));
        key.set_xalign(1.0);
        key.add_css_class("postio-keyhint");
        grid.attach(&key, 1, line, 1, 1);

        // Read as one sentence rather than as two stray fragments.
        title.update_property(&[gtk::accessible::Property::Label(&spoken(row))]);
        key.set_accessible_role(gtk::AccessibleRole::Presentation);
    }
    group.append(&grid);
    group
}
