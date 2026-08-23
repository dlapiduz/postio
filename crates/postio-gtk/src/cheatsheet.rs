//! The `?` cheat sheet.
//!
//! # Generated, so it cannot be wrong
//!
//! A hand-written key reference is out of date by the next release — that is
//! the whole reason [`postio_core::registry`] exists (spec.md §8: one table,
//! every surface). This sheet is rendered from that table and from the live
//! [`Keymap`], so rebinding a key in `config.toml` changes what the overlay
//! says without anybody editing a list.
//!
//! # Two halves
//!
//! [`sections`] decides what the sheet contains and is a pure function, tested
//! with no display. [`CheatSheet`] is the widget around it.
//!
//! # Grouping
//!
//! Every command lands in exactly one section, even though most are reachable
//! in several contexts. A reader scanning for "how do I archive" wants one
//! answer, not the same row under *List*, *Thread* and *Reader*; so a command
//! reachable everywhere goes under **Everywhere**, and anything else goes under
//! the first context it applies in, which is also the broadest. That keeps the
//! sheet the length of the registry rather than the length of the registry
//! times six.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::{CommandId, Context, ContextSet, Keymap, registry};

/// One key the sheet lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The command it runs.
    pub id: CommandId,
    /// Its title, as the registry gives it.
    pub title: &'static str,
    /// The binding in force. `None` means palette-only — still worth listing,
    /// because "this exists and has no key" is an answer.
    pub binding: Option<String>,
}

/// A group of keys under one heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The heading, as the sheet prints it.
    pub title: &'static str,
    /// Its keys, in registry order.
    pub rows: Vec<Row>,
}

/// The heading a command reachable in every context is filed under.
const EVERYWHERE: &str = "Everywhere";

/// The heading for each context, in the order the sheet lays them out.
fn heading(context: Context) -> &'static str {
    match context {
        Context::List => "Message list",
        Context::Thread => "Thread",
        Context::Reader => "Reading",
        Context::Composer => "Composing",
        Context::Search => "Search",
        Context::Palette => "Command palette",
    }
}

/// The whole sheet, in the order it is laid out.
///
/// Empty sections are dropped: a heading with nothing under it is worse than no
/// heading, and which sections have content depends on what the registry holds.
pub fn sections(keymap: &Keymap) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    let mut everywhere = Section {
        title: EVERYWHERE,
        rows: Vec::new(),
    };

    for context in Context::ALL {
        sections.push(Section {
            title: heading(*context),
            rows: Vec::new(),
        });
    }

    for spec in registry::all() {
        let row = Row {
            id: spec.id,
            title: spec.title,
            binding: keymap.binding(spec.id).map(str::to_owned),
        };

        if spec.contexts == ContextSet::ANY {
            everywhere.rows.push(row);
            continue;
        }
        // The first context it applies in, which is also the broadest: filing
        // "Archive" under *Message list* rather than repeating it under
        // *Thread* and *Reading* as well.
        let Some(first) = spec.contexts.iter().next() else {
            continue;
        };
        let heading = heading(first);
        if let Some(section) = sections.iter_mut().find(|s| s.title == heading) {
            section.rows.push(row);
        }
    }

    let mut out = Vec::with_capacity(sections.len() + 1);
    if !everywhere.rows.is_empty() {
        out.push(everywhere);
    }
    out.extend(sections.into_iter().filter(|s| !s.rows.is_empty()));
    out
}

/// How a row reads to a screen reader.
///
/// The visual row is a title and a key set apart from each other; read aloud
/// that becomes "Archive a", which is not a sentence. This is.
pub fn spoken(row: &Row) -> String {
    match &row.binding {
        Some(binding) => format!("{}, press {binding}", row.title),
        None => format!("{}, no keyboard shortcut", row.title),
    }
}

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct CheatSheet {
        pub columns: gtk::Box,
        pub keymap: RefCell<Keymap>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
    }

    impl Default for CheatSheet {
        fn default() -> Self {
            Self {
                columns: gtk::Box::new(gtk::Orientation::Horizontal, 32),
                keymap: RefCell::new(Keymap::default()),
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

    /// What the sheet currently lists.
    pub fn sections(&self) -> Vec<Section> {
        sections(&self.imp().keymap.borrow())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Keymap {
        Keymap::resolve(&postio_config::KeyBindings::default())
    }

    #[test]
    fn every_command_appears_exactly_once() {
        let listed: Vec<CommandId> = sections(&defaults())
            .into_iter()
            .flat_map(|section| section.rows)
            .map(|row| row.id)
            .collect();

        for spec in registry::all() {
            let count = listed.iter().filter(|id| **id == spec.id).count();
            assert_eq!(
                count, 1,
                "`{}` appears {count} times; a reader wants one answer",
                spec.id
            );
        }
        assert_eq!(listed.len(), registry::all().count());
    }

    #[test]
    fn a_command_reachable_everywhere_is_filed_under_everywhere() {
        let sections = sections(&defaults());
        let everywhere = sections
            .iter()
            .find(|section| section.title == EVERYWHERE)
            .expect("an Everywhere section");

        let ids: Vec<CommandId> = everywhere.rows.iter().map(|row| row.id).collect();
        assert!(ids.contains(&CommandId::CommandPalette));
        assert!(ids.contains(&CommandId::Back));
        assert_eq!(
            sections.first().map(|section| section.title),
            Some(EVERYWHERE),
            "and it leads, because it is what applies wherever the reader is"
        );
    }

    #[test]
    fn a_command_is_filed_under_the_broadest_context_it_applies_in() {
        let sections = sections(&defaults());
        let list = sections
            .iter()
            .find(|section| section.title == heading(Context::List))
            .expect("a Message list section");

        assert!(
            list.rows.iter().any(|row| row.id == CommandId::Archive),
            "archive works in the list, the thread and the reader; the sheet \
             says so once"
        );

        let composing = sections
            .iter()
            .find(|section| section.title == heading(Context::Composer))
            .expect("a Composing section");
        assert!(composing.rows.iter().any(|row| row.id == CommandId::Send));
    }

    #[test]
    fn no_section_is_printed_empty() {
        for section in sections(&defaults()) {
            assert!(
                !section.rows.is_empty(),
                "`{}` has no keys under it",
                section.title
            );
        }
    }

    #[test]
    fn rows_carry_the_binding_in_force() {
        let archive = sections(&defaults())
            .into_iter()
            .flat_map(|section| section.rows)
            .find(|row| row.id == CommandId::Archive)
            .expect("archive");

        assert_eq!(archive.binding.as_deref(), Some("a"));
    }

    #[test]
    fn rebinding_a_key_changes_the_sheet() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_owned(), "y".to_owned());

        let archive = sections(&Keymap::resolve(&overrides))
            .into_iter()
            .flat_map(|section| section.rows)
            .find(|row| row.id == CommandId::Archive)
            .expect("archive");

        assert_eq!(
            archive.binding.as_deref(),
            Some("y"),
            "with no code edit: the sheet reads the live keymap"
        );
    }

    #[test]
    fn a_command_with_no_key_is_still_listed() {
        // An empty keymap is what a build with every binding taken looks like.
        let listed: Vec<Row> = sections(&Keymap::default())
            .into_iter()
            .flat_map(|section| section.rows)
            .collect();

        assert_eq!(listed.len(), registry::all().count());
        assert!(
            listed.iter().all(|row| row.binding.is_none()),
            "and \"this exists and has no key\" is an answer worth printing"
        );
    }

    #[test]
    fn a_row_reads_as_a_sentence() {
        assert_eq!(
            spoken(&Row {
                id: CommandId::Archive,
                title: "Archive",
                binding: Some("a".to_owned()),
            }),
            "Archive, press a"
        );
        assert_eq!(
            spoken(&Row {
                id: CommandId::Archive,
                title: "Archive",
                binding: None,
            }),
            "Archive, no keyboard shortcut"
        );
    }

    #[test]
    fn the_sections_are_the_ones_the_registry_actually_uses() {
        let titles: Vec<&str> = sections(&defaults())
            .iter()
            .map(|section| section.title)
            .collect();

        assert_eq!(
            titles,
            vec![
                EVERYWHERE,
                heading(Context::List),
                heading(Context::Composer)
            ],
            "no registry command is filed under Thread, Reading, Search or \
             Palette today; if one is, this test is how you find out"
        );
    }
}
