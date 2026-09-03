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
//! [`sections`] decides what the sheet contains and is a pure function, tested
//! with no display. [`CheatSheet`] is the widget around it.
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
use postio_core::{ActionId, Context, ContextSet, Keymap, Scope, registry};

use crate::finder::Mode;

/// One key the sheet lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The command it runs, when it runs one.
    ///
    /// `None` for the one box's prefixes: `>` and `#` and `@` are not
    /// commands, they are what you type *inside* a surface a command opened.
    /// They belong on the sheet all the same — a user who never reads docs
    /// would otherwise never discover two thirds of the box.
    ///
    /// An [`ActionId`], so a command registered at runtime is taught here on
    /// the same footing as a built-in. `?` is half of what makes this app
    /// learnable, and a command absent from it is one the user will not find.
    pub id: Option<ActionId>,
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

/// The heading the one box's prefixes are filed under.
const IN_THE_BOX: &str = "In the search box";

/// The heading for each context, in the order the sheet lays them out.
fn heading(context: Context) -> &'static str {
    match context {
        Context::List => "Message list",
        Context::Thread => "Thread",
        Context::Reader => "Reading",
        Context::Composer => "Composing",
        Context::Search => "Search",
        Context::Palette => "Command palette",
        // "Folders" rather than "Sidebar": the vocabulary table says folder
        // in UI copy, and the section is about what is in the pane, not about
        // the pane.
        Context::Sidebar => "Folders",
        Context::Parts => "Parts",
        // "Accounts" rather than "Settings": the section is about the account
        // list, which is the only part of the settings panel these keys reach
        // (ADR 0005 Q6c).
        Context::Accounts => "Accounts",
        // Same reasoning as `Accounts`, one section down: these keys reach
        // only the keybinding list, not the whole settings panel.
        Context::Keys => "Keybindings",
    }
}

/// The whole sheet, in the order it is laid out.
///
/// Empty sections are dropped: a heading with nothing under it is worse than no
/// heading, and which sections have content depends on what the registry holds.
pub fn sections(keymap: &Keymap, context: Context, scope: Scope) -> Vec<Section> {
    let mut everywhere = Section {
        title: EVERYWHERE,
        rows: Vec::new(),
    };
    let mut here = Section {
        title: heading(context),
        rows: Vec::new(),
    };

    for spec in registry::reachable_in(context, scope) {
        let ActionId::Builtin(id) = spec.id else {
            // Registered commands get their own sections, by provenance.
            continue;
        };
        let row = Row {
            id: Some(spec.id),
            title: spec.title,
            binding: keymap.binding(id).map(str::to_owned),
        };

        if spec.contexts == ContextSet::ANY {
            everywhere.rows.push(row);
        } else {
            here.rows.push(row);
        }
    }

    let sections = vec![here];

    let mut out = Vec::with_capacity(sections.len() + 2);
    if !everywhere.rows.is_empty() {
        out.push(everywhere);
    }
    // Right after the keys that *open* the box, so the sheet reads "here is
    // how to get there" and then "here is what you can type once you are".
    if let Some(prefixes) = prefix_section() {
        out.push(prefixes);
    }
    out.extend(sections.into_iter().filter(|s| !s.rows.is_empty()));
    out.extend(extension_sections(keymap));
    out
}

/// One section per namespace, for the commands registered at runtime.
///
/// Grouped by where they came from rather than folded into the built-in
/// sections by context, because provenance is the thing a user needs here:
/// `?` answers "what can I do", and "this came from the MCP server you
/// connected" is part of the answer in a way that "this works in the message
/// list" already is for a built-in.
///
/// Last, and never interleaved. The built-in sheet is a stable thing people
/// learn; a plugin must not be able to reorder it by registering early.
fn extension_sections(keymap: &Keymap) -> Vec<Section> {
    let mut sections: Vec<Section> = Vec::new();
    for spec in registry::every_action() {
        let ActionId::Ext(id) = spec.id else {
            continue;
        };
        let row = Row {
            id: Some(spec.id),
            title: spec.title,
            binding: keymap.binding(spec.id).map(str::to_owned),
        };
        match sections
            .iter_mut()
            .find(|section| section.title == id.namespace())
        {
            Some(section) => section.rows.push(row),
            None => sections.push(Section {
                title: id.namespace(),
                rows: vec![row],
            }),
        }
    }
    sections
}

/// What the one box does with a leading `>`, `#` or `@`.
///
/// Generated from [`Mode::ALL`], which has always been documented as being in
/// the order the sheet should list them and which until now nothing read. A
/// mode added later appears here with no other edit — that is the whole point,
/// and it is the acceptance criterion of `postio-2ee`.
///
/// [`Mode::Search`] is skipped: it has no prefix because it is what the box
/// already is, and the key that opens it is `/`, which the registry lists.
fn prefix_section() -> Option<Section> {
    let rows: Vec<Row> = Mode::ALL
        .into_iter()
        .filter_map(|mode| {
            Some(Row {
                id: None,
                title: mode.placeholder(),
                binding: Some(mode.prefix()?.to_string()),
            })
        })
        .collect();
    (!rows.is_empty()).then_some(Section {
        title: IN_THE_BOX,
        rows,
    })
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
        /// Where the reader is standing, and what is on screen. The sheet
        /// lists what is reachable from there rather than the whole
        /// vocabulary (#182).
        pub context: RefCell<Context>,
        pub scope: RefCell<Scope>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
    }

    impl Default for CheatSheet {
        fn default() -> Self {
            Self {
                columns: gtk::Box::new(gtk::Orientation::Horizontal, 32),
                keymap: RefCell::new(Keymap::default()),
                context: RefCell::new(Context::List),
                scope: RefCell::new(Scope::default()),
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

    /// What the mail on screen belongs to — see [`Scope`].
    pub fn set_scope(&self, scope: Scope) {
        *self.imp().scope.borrow_mut() = scope;
        self.rebuild();
    }

    /// What the sheet currently lists.
    pub fn sections(&self) -> Vec<Section> {
        sections(
            &self.imp().keymap.borrow(),
            *self.imp().context.borrow(),
            *self.imp().scope.borrow(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use postio_core::{CommandId, Scope};
    use postio_model::AccountId;

    fn defaults() -> Keymap {
        Keymap::resolve(&postio_config::KeyBindings::default())
    }

    /// The reader's ordinary position: standing in the message list, with one
    /// account's mailboxes on screen.
    fn in_the_list() -> (Context, Scope) {
        (Context::List, Scope::Account(AccountId::new(1)))
    }

    #[test]
    fn every_reachable_command_appears_exactly_once() {
        let (context, scope) = in_the_list();
        let listed: Vec<ActionId> = sections(&defaults(), context, scope)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter_map(|row| row.id)
            .collect();

        for spec in registry::reachable_in(context, scope) {
            let count = listed.iter().filter(|id| **id == spec.id).count();
            assert_eq!(
                count, 1,
                "`{}` appears {count} times; a reader wants one answer",
                spec.id
            );
        }
        assert_eq!(listed.len(), registry::reachable_in(context, scope).count());
    }

    /// The sheet answers "what can I do *now*", so it must not advertise a
    /// key that would do nothing where the reader is standing.
    #[test]
    fn the_sheet_lists_only_what_is_reachable_here() {
        let (context, scope) = in_the_list();
        let listed: Vec<ActionId> = sections(&defaults(), context, scope)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter_map(|row| row.id)
            .collect();

        assert!(
            listed.contains(&ActionId::Builtin(CommandId::Archive)),
            "archiving is the list's whole job"
        );
        assert!(
            !listed.contains(&ActionId::Builtin(CommandId::Send)),
            "sending belongs to the composer; offering it here teaches a key \
             that does nothing"
        );
        assert!(
            !listed.contains(&ActionId::Builtin(CommandId::ToggleThreadUnread)),
            "the thread filter needs a thread column on screen"
        );
    }

    /// #182's acceptance, on the second surface it names.
    #[test]
    fn a_unified_view_lists_no_move() {
        let ids = |scope| -> Vec<ActionId> {
            sections(&defaults(), Context::List, scope)
                .into_iter()
                .flat_map(|section| section.rows)
                .filter_map(|row| row.id)
                .collect()
        };

        assert!(
            ids(Scope::Account(AccountId::new(1))).contains(&ActionId::Builtin(CommandId::Move)),
            "an account view can name a destination"
        );
        assert!(
            !ids(Scope::Unified).contains(&ActionId::Builtin(CommandId::Move)),
            "a unified view cannot, so the sheet must not teach `m` there"
        );
    }

    #[test]
    fn a_command_reachable_everywhere_is_filed_under_everywhere() {
        let (context, scope) = in_the_list();
        let sections = sections(&defaults(), context, scope);
        let everywhere = sections
            .iter()
            .find(|section| section.title == EVERYWHERE)
            .expect("an Everywhere section");

        let ids: Vec<ActionId> = everywhere.rows.iter().filter_map(|row| row.id).collect();
        assert!(ids.contains(&ActionId::Builtin(CommandId::CommandPalette)));
        assert!(ids.contains(&ActionId::Builtin(CommandId::Back)));
        assert_eq!(
            sections.first().map(|section| section.title),
            Some(EVERYWHERE),
            "and it leads, because it is what applies wherever the reader is"
        );
    }

    #[test]
    fn a_reachable_command_is_filed_under_the_surface_the_reader_is_on() {
        let from_the_list = sections(&defaults(), in_the_list().0, in_the_list().1);
        let list = from_the_list
            .iter()
            .find(|section| section.title == heading(Context::List))
            .expect("a Message list section");

        assert!(
            list.rows
                .iter()
                .any(|row| row.id == Some(ActionId::Builtin(CommandId::Archive))),
            "archive works in the list, the thread and the reader; standing in \
             the list, the sheet says so under the list"
        );

        // The same command, read from a different surface, files under that
        // surface — which is the whole reason this stopped keying off the
        // command's own first context.
        let from_the_reader = sections(&defaults(), Context::Reader, in_the_list().1);
        let reading = from_the_reader
            .iter()
            .find(|section| section.title == heading(Context::Reader))
            .expect("a Reading section");
        assert!(
            reading
                .rows
                .iter()
                .any(|row| row.id == Some(ActionId::Builtin(CommandId::Archive))),
            "the heading has to name where the reader is, not where the \
             command was first declared"
        );
    }

    #[test]
    fn no_section_is_printed_empty() {
        for section in sections(&defaults(), in_the_list().0, in_the_list().1) {
            assert!(
                !section.rows.is_empty(),
                "`{}` has no keys under it",
                section.title
            );
        }
    }

    #[test]
    fn rows_carry_the_binding_in_force() {
        let archive = sections(&defaults(), in_the_list().0, in_the_list().1)
            .into_iter()
            .flat_map(|section| section.rows)
            .find(|row| row.id == Some(ActionId::Builtin(CommandId::Archive)))
            .expect("archive");

        assert_eq!(archive.binding.as_deref(), Some("a"));
    }

    #[test]
    fn rebinding_a_key_changes_the_sheet() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_owned(), "y".to_owned());

        let archive = sections(
            &Keymap::resolve(&overrides),
            in_the_list().0,
            in_the_list().1,
        )
        .into_iter()
        .flat_map(|section| section.rows)
        .find(|row| row.id == Some(ActionId::Builtin(CommandId::Archive)))
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
        //
        // Commands only: the box's prefixes are also rows, but `>` is not a
        // binding the keymap has any say over — it is a character you type
        // into a surface — so an empty keymap does not silence one.
        let listed: Vec<Row> = sections(&Keymap::default(), in_the_list().0, in_the_list().1)
            .into_iter()
            .flat_map(|section| section.rows)
            .filter(|row| row.id.is_some())
            .collect();

        assert_eq!(
            listed.len(),
            registry::reachable_in(in_the_list().0, in_the_list().1).count()
        );
        assert!(
            listed.iter().all(|row| row.binding.is_none()),
            "and \"this exists and has no key\" is an answer worth printing"
        );
    }

    #[test]
    fn a_row_reads_as_a_sentence() {
        assert_eq!(
            spoken(&Row {
                id: Some(ActionId::Builtin(CommandId::Archive)),
                title: "Archive",
                binding: Some("a".to_owned()),
            }),
            "Archive, press a"
        );
        assert_eq!(
            spoken(&Row {
                id: Some(ActionId::Builtin(CommandId::Archive)),
                title: "Archive",
                binding: None,
            }),
            "Archive, no keyboard shortcut"
        );
    }

    #[test]
    fn the_sheet_teaches_every_prefix_the_one_box_understands() {
        // `postio-2ee`: `?` is generated from the command registry, and the
        // box's prefixes are not commands — so `>`, `#` and `@` appeared
        // nowhere in the app's own teaching. A user who never reads docs
        // would never discover two thirds of the box.
        let section = sections(&defaults(), in_the_list().0, in_the_list().1)
            .into_iter()
            .find(|section| section.title == IN_THE_BOX)
            .expect("the sheet has no section for the box's prefixes");

        // Driven from `Mode::ALL` rather than a written-out list, so adding a
        // mode makes this test cover it without anyone editing the test —
        // which is the same property the section itself has to have.
        for mode in Mode::ALL {
            let Some(prefix) = mode.prefix() else {
                continue;
            };
            let row = section
                .rows
                .iter()
                .find(|row| row.binding.as_deref() == Some(&prefix.to_string()))
                .unwrap_or_else(|| panic!("`{prefix}` is not on the sheet"));
            assert_eq!(
                row.title,
                mode.placeholder(),
                "`{prefix}` is listed as something other than what it does"
            );
            assert!(
                row.id.is_none(),
                "a prefix is not a command and must not claim to run one"
            );
        }

        assert_eq!(
            section.rows.len(),
            Mode::ALL
                .into_iter()
                .filter(|m| m.prefix().is_some())
                .count(),
            "the section is not generated from Mode::ALL — a mode added later \
             would not appear"
        );
    }

    #[test]
    fn search_is_not_listed_as_a_prefix_because_it_has_none() {
        // It is what the box already is; the key that opens it is `/`, which
        // the registry lists as a command.
        assert_eq!(Mode::Search.prefix(), None);
        let section = sections(&defaults(), in_the_list().0, in_the_list().1)
            .into_iter()
            .find(|section| section.title == IN_THE_BOX)
            .expect("a prefix section");
        assert!(
            !section
                .rows
                .iter()
                .any(|row| row.title == Mode::Search.placeholder()),
            "search is listed as though it needed a prefix typed to reach it"
        );
    }

    #[test]
    fn the_sheet_prints_only_everywhere_and_the_readers_own_surface() {
        for context in [Context::List, Context::Composer, Context::Sidebar] {
            let titles: Vec<&str> = sections(&defaults(), context, in_the_list().1)
                .iter()
                .map(|section| section.title)
                .collect();

            let expected: Vec<&str> = [EVERYWHERE, IN_THE_BOX, heading(context)]
                .into_iter()
                .filter(|title| titles.contains(title))
                .collect();
            assert_eq!(
                titles, expected,
                "after filtering to what is reachable there are only two \
                 useful headings — what works everywhere, and what works \
                 here. A third means the filter let something through."
            );
        }
    }
}
