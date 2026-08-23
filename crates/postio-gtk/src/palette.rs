//! The `Ctrl+K` command palette.
//!
//! # Why it exists
//!
//! spec.md §8 asks that every command be reachable without memorizing a
//! binding. The palette is that promise kept: it is generated from
//! [`postio_core::registry`], so a command that exists is a command that can be
//! found, and one that gains a binding shows it here without anybody editing a
//! second list.
//!
//! # Two halves
//!
//! Everything that decides *what to show* — the fuzzy match, the ranking, the
//! context filter, the binding each row displays — is in [`entries`] and
//! [`score`], which are pure functions over a [`postio_core::Keymap`]. They are
//! unit-tested with no display and no GTK main loop, which is also what makes
//! the 16 ms budget measurable rather than a hope.
//!
//! [`Palette`] is the widget around them. It rebuilds its rows from `entries`
//! on every keystroke, which is affordable because the registry is a few dozen
//! rows: no incremental diffing, no stale state, nothing to get out of step.
//!
//! # What the rows say
//!
//! Title, then the binding in `IBM Plex Mono` on the right — the same
//! arrangement the canvas uses for the key hints on a focused row, so a key
//! learned in the palette looks the same when it appears in the list.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::{CommandId, Context, Keymap, registry};

/// How many rows the palette will show at once.
///
/// The list scrolls past this; the cap is on what is *built*, so a query that
/// matches everything costs the same as one that matches three things.
pub const MAX_ROWS: usize = 32;

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Where a query matched, and how well.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    /// Higher is better. Only comparable between candidates for one query.
    pub score: i32,
    /// Byte indices in the candidate that the query matched, ascending.
    pub positions: Vec<usize>,
}

/// A match at the start of a word is worth far more than one mid-word: typing
/// `cp` should find "Command palette" ahead of "Copy".
const WORD_START: i32 = 24;
/// Each character that continues an unbroken run.
const CONSECUTIVE: i32 = 12;
/// Every character skipped over, up to a floor — a match spread across a long
/// title is still a match, just a worse one.
const GAP: i32 = -2;
/// The most any single gap can cost.
const GAP_FLOOR: i32 = -12;

/// Scores `query` as a subsequence of `candidate`, case-insensitively.
///
/// Returns `None` when the query is not a subsequence at all. An empty query
/// matches everything with a score of zero, which is what makes an
/// unfiltered palette fall back to registry order.
///
/// Greedy left-to-right: the first place each character can go is where it
/// goes. That is not optimal scoring — a backtracking matcher would find that
/// `cp` scores better against "Command **p**alette" than "**C**ommand
/// **p**alette" — but it is linear, and for a few dozen short titles the
/// difference is invisible while the cost of getting it wrong at scale is not.
pub fn score(query: &str, candidate: &str) -> Option<Match> {
    if query.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }

    let mut positions = Vec::new();
    let mut total = 0;
    let mut run = 0;
    let mut last: Option<usize> = None;
    let mut haystack = candidate.char_indices().peekable();

    for wanted in query.chars() {
        let wanted = wanted.to_ascii_lowercase();
        let mut found = None;
        for (index, character) in haystack.by_ref() {
            if character.to_ascii_lowercase() == wanted {
                found = Some(index);
                break;
            }
        }
        let index = found?;

        if last.is_some_and(|previous| previous + 1 == index) {
            run += 1;
            total += CONSECUTIVE * run.min(3);
        } else {
            run = 0;
            if starts_a_word(candidate, index) {
                total += WORD_START;
            }
            if let Some(previous) = last {
                let skipped = (index - previous - 1) as i32;
                total += (GAP * skipped).max(GAP_FLOOR);
            } else {
                // Leading noise before the first match is worth less than a gap
                // inside it: "message" matching "Next message" is fine.
                total += (GAP * index as i32).max(GAP_FLOOR);
            }
        }

        positions.push(index);
        last = Some(index);
    }

    // Between two candidates that match equally well, the shorter one is the
    // one the user meant.
    total -= candidate.len() as i32 / 8;

    Some(Match {
        score: total,
        positions,
    })
}

/// Whether the byte at `index` begins a word.
fn starts_a_word(candidate: &str, index: usize) -> bool {
    if index == 0 {
        return true;
    }
    candidate[..index]
        .chars()
        .next_back()
        .is_some_and(|previous| !previous.is_alphanumeric())
}

// ---------------------------------------------------------------------------
// Entries
// ---------------------------------------------------------------------------

/// One row of the palette.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The command this row runs.
    pub id: CommandId,
    /// Its title, as the registry gives it.
    pub title: &'static str,
    /// Its binding in force, or `None` when it is palette-only.
    ///
    /// Read from the live [`Keymap`], so a `[keys]` override shows here without
    /// anything else being told.
    pub binding: Option<String>,
    /// Byte indices in `title` the query matched, for highlighting.
    pub positions: Vec<usize>,
    /// How well it matched. Rows come out highest first.
    pub score: i32,
}

/// Penalty for matching the stable id rather than the title.
///
/// `archive_thread` is findable by typing `archive_th`, but a title match for
/// the same query should always rank above it.
const ID_PENALTY: i32 = 40;

/// The rows to show for `query`, best first.
///
/// Filtered to commands reachable in `context` — offering to send a draft from
/// the message list is a row the user can only be disappointed by. An empty
/// query returns everything applicable, in registry order, which is the order
/// the cheat sheet uses.
pub fn entries(keymap: &Keymap, context: Context, query: &str) -> Vec<Entry> {
    let query = query.trim();
    let mut found: Vec<Entry> = registry::for_context(context)
        .filter_map(|spec| {
            let by_title = score(query, spec.title);
            let by_id = score(query, spec.id.as_str());
            let matched = match (by_title, by_id) {
                (Some(title), _) => title,
                (None, Some(id)) => Match {
                    score: id.score - ID_PENALTY,
                    positions: Vec::new(),
                },
                (None, None) => return None,
            };
            Some(Entry {
                id: spec.id,
                title: spec.title,
                binding: keymap.binding(spec.id).map(str::to_owned),
                positions: matched.positions,
                score: matched.score,
            })
        })
        .collect();

    // Stable, so an empty query — every score zero — comes out in registry
    // order rather than in whatever order the sort happened to leave it.
    found.sort_by_key(|entry| std::cmp::Reverse(entry.score));
    found.truncate(MAX_ROWS);
    found
}

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

/// What to call when the user picks a command.
type ActivateHandler = Box<dyn Fn(CommandId)>;

mod imp {
    use std::cell::RefCell;

    use super::*;

    pub struct Palette {
        pub search: gtk::SearchEntry,
        pub list: gtk::ListBox,
        pub empty: gtk::Label,
        pub scroller: gtk::ScrolledWindow,
        pub keymap: RefCell<Keymap>,
        pub context: RefCell<Context>,
        pub shown: RefCell<Vec<CommandId>>,
        pub activated: RefCell<Vec<ActivateHandler>>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
    }

    impl Default for Palette {
        fn default() -> Self {
            Self {
                search: gtk::SearchEntry::new(),
                list: gtk::ListBox::new(),
                empty: gtk::Label::new(None),
                scroller: gtk::ScrolledWindow::new(),
                keymap: RefCell::new(Keymap::default()),
                context: RefCell::new(Context::List),
                shown: RefCell::new(Vec::new()),
                activated: RefCell::new(Vec::new()),
                dismissed: RefCell::new(Vec::new()),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Palette {
        const NAME: &'static str = "PostioPalette";
        type Type = super::Palette;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Palette {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }
    }

    impl WidgetImpl for Palette {}
    impl BinImpl for Palette {}
}

glib::wrapper! {
    /// The `Ctrl+K` overlay: type, pick, run.
    pub struct Palette(ObjectSubclass<imp::Palette>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Palette {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Palette {
    /// An empty palette over the registry defaults.
    pub fn new() -> Self {
        Self::default()
    }

    /// The bindings to show beside each command.
    ///
    /// Call this whenever `[keys]` changes; the rows are rebuilt from it.
    pub fn set_keymap(&self, keymap: Keymap) {
        *self.imp().keymap.borrow_mut() = keymap;
        self.refresh();
    }

    /// The context to filter commands by.
    pub fn set_context(&self, context: Context) {
        *self.imp().context.borrow_mut() = context;
        self.refresh();
    }

    /// The commands currently listed, best first.
    pub fn visible(&self) -> Vec<CommandId> {
        self.imp().shown.borrow().clone()
    }

    /// The query as typed.
    pub fn query(&self) -> String {
        self.imp().search.text().to_string()
    }

    /// Types a query, as though the user had.
    pub fn set_query(&self, query: &str) {
        self.imp().search.set_text(query);
    }

    /// The row the user would run by pressing Enter.
    pub fn selected(&self) -> Option<CommandId> {
        let index = self.imp().list.selected_row()?.index();
        self.imp().shown.borrow().get(index as usize).copied()
    }

    /// Moves the selection by `delta` rows, stopping at either end.
    ///
    /// The palette owns arrow-key navigation rather than letting the list box
    /// have it, because focus stays in the search entry the whole time — the
    /// user is still typing.
    pub fn move_selection(&self, delta: i32) {
        let count = self.imp().shown.borrow().len() as i32;
        if count == 0 {
            return;
        }
        let current = self
            .imp()
            .list
            .selected_row()
            .map(|row| row.index())
            .unwrap_or(0);
        let next = (current + delta).clamp(0, count - 1);
        if let Some(row) = self.imp().list.row_at_index(next) {
            self.imp().list.select_row(Some(&row));
            row.grab_focus();
            self.imp().search.grab_focus();
        }
    }

    /// Runs the selected command, if there is one.
    pub fn activate_selected(&self) {
        let Some(id) = self.selected() else {
            return;
        };
        for handler in self.imp().activated.borrow().iter() {
            handler(id);
        }
    }

    /// Called with the command the user picked.
    pub fn connect_activated(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().activated.borrow_mut().push(Box::new(handler));
    }

    /// Called when the user presses `Escape`.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.imp().dismissed.borrow_mut().push(Box::new(handler));
    }

    /// Clears the query and puts the cursor in the entry.
    ///
    /// What `Ctrl+K` does. The query is *not* remembered between openings: a
    /// palette that reopens showing the last search is one the user has to
    /// clear before they can use it.
    pub fn focus_search(&self) {
        self.imp().search.set_text("");
        self.imp().search.grab_focus();
        self.refresh();
    }

    fn dismiss(&self) {
        for handler in self.imp().dismissed.borrow().iter() {
            handler();
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-palette");

        imp.search.set_placeholder_text(Some("Run a command…"));
        imp.search.add_css_class("postio-search");

        imp.list.set_selection_mode(gtk::SelectionMode::Single);
        imp.list.add_css_class("postio-palette-list");
        imp.list.set_activate_on_single_click(true);

        imp.empty.set_label("No matching command");
        imp.empty.add_css_class("postio-palette-empty");
        imp.empty.set_visible(false);

        imp.scroller.set_child(Some(&imp.list));
        imp.scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.scroller.set_vexpand(true);
        imp.scroller.set_max_content_height(360);
        imp.scroller.set_propagate_natural_height(true);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&imp.search);
        column.append(&imp.empty);
        column.append(&imp.scroller);
        self.set_child(Some(&column));
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Start);

        // `changed`, not `search-changed`: the latter debounces by 150 ms,
        // which is right for a search that hits a database and wrong for a list
        // of thirty rows already in memory. The bead asks that it filter
        // instantly, and instantly is what a keystroke costs here.
        imp.search.connect_changed(glib::clone!(
            #[weak(rename_to = palette)]
            self,
            move |_| palette.refresh()
        ));

        imp.list.connect_row_activated(glib::clone!(
            #[weak(rename_to = palette)]
            self,
            move |_, row| {
                let index = row.index() as usize;
                let id = palette.imp().shown.borrow().get(index).copied();
                if let Some(id) = id {
                    for handler in palette.imp().activated.borrow().iter() {
                        handler(id);
                    }
                }
            }
        ));

        // The search entry keeps focus, so the keys that drive the list have to
        // be caught before it consumes them.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = palette)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| match key {
                gtk::gdk::Key::Down => {
                    palette.move_selection(1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Up => {
                    palette.move_selection(-1);
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter => {
                    palette.activate_selected();
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::Escape => {
                    palette.dismiss();
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        ));
        imp.search.add_controller(keys);

        self.refresh();
    }

    /// Rebuilds the rows from the current query.
    ///
    /// Whole-list rebuild on every keystroke: the registry is a few dozen rows,
    /// so this is microseconds, and it cannot leave a stale row behind the way
    /// an incremental update can.
    fn refresh(&self) {
        let imp = self.imp();
        let query = imp.search.text().to_string();
        let found = entries(&imp.keymap.borrow(), *imp.context.borrow(), &query);

        while let Some(row) = imp.list.first_child() {
            imp.list.remove(&row);
        }
        for entry in &found {
            imp.list.append(&row_for(entry));
        }

        let empty = found.is_empty();
        imp.empty.set_visible(empty);
        imp.scroller.set_visible(!empty);

        *imp.shown.borrow_mut() = found.iter().map(|entry| entry.id).collect();
        if let Some(first) = imp.list.row_at_index(0) {
            imp.list.select_row(Some(&first));
        }
    }
}

fn row_for(entry: &Entry) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-palette-row");

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let title = gtk::Label::new(None);
    title.set_markup(&highlight(entry.title, &entry.positions));
    title.set_xalign(0.0);
    title.set_hexpand(true);
    title.add_css_class("postio-palette-title");
    line.append(&title);

    if let Some(binding) = &entry.binding {
        let hint = gtk::Label::new(Some(binding));
        hint.add_css_class("postio-keyhint");
        line.append(&hint);
    }

    row.set_child(Some(&line));
    row.set_tooltip_text(Some(entry.id.as_str()));
    row
}

/// Bolds the characters the query matched.
fn highlight(title: &str, positions: &[usize]) -> String {
    let mut out = String::with_capacity(title.len() + positions.len() * 7);
    for (index, character) in title.char_indices() {
        let escaped = glib::markup_escape_text(&character.to_string());
        if positions.contains(&index) {
            out.push_str("<b>");
            out.push_str(&escaped);
            out.push_str("</b>");
        } else {
            out.push_str(&escaped);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn defaults() -> Keymap {
        Keymap::resolve(&postio_config::KeyBindings::default())
    }

    // -- matching ---------------------------------------------------------

    #[test]
    fn a_query_that_is_not_a_subsequence_does_not_match() {
        assert!(score("zzz", "Archive").is_none());
        assert!(score("ahcrive", "Archive").is_none(), "order matters");
    }

    #[test]
    fn an_empty_query_matches_everything_equally() {
        assert_eq!(score("", "Archive").expect("a match").score, 0);
        assert_eq!(score("", "Reply to all").expect("a match").score, 0);
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(score("ARCH", "Archive").is_some());
        assert!(score("arch", "Archive").is_some());
    }

    #[test]
    fn the_positions_are_where_it_matched() {
        let matched = score("arc", "Archive").expect("a match");

        assert_eq!(matched.positions, vec![0, 1, 2]);
    }

    #[test]
    fn word_initials_beat_a_run_buried_mid_word() {
        let initials = score("ra", "Reply to all").expect("a match").score;
        let buried = score("ra", "Forward").expect("a match").score;

        assert!(
            initials > buried,
            "initials {initials} should beat mid-word {buried}"
        );
    }

    #[test]
    fn a_prefix_beats_the_same_letters_further_in() {
        // Synthetic rather than two registry titles: what is being pinned is the
        // scoring rule, and a real pair would also differ in length and word
        // boundaries.
        let prefix = score("com", "compose").expect("a match").score;
        let later = score("com", "recompose").expect("a match").score;

        assert!(prefix > later, "{prefix} vs {later}");
    }

    #[test]
    fn between_equal_matches_the_shorter_title_wins() {
        let short = score("f", "Flag").expect("a match").score;
        let long = score("f", "Forward the whole conversation")
            .expect("a match")
            .score;

        assert!(short > long, "{short} vs {long}");
    }

    // -- entries ----------------------------------------------------------

    #[test]
    fn an_empty_query_lists_everything_reachable_in_registry_order() {
        let listed = entries(&defaults(), Context::List, "");
        let expected: Vec<CommandId> = registry::for_context(Context::List)
            .map(|spec| spec.id)
            .collect();

        assert_eq!(
            listed.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            expected
        );
    }

    #[test]
    fn every_registry_command_is_reachable_from_some_context() {
        for spec in registry::all() {
            let reachable = Context::ALL.iter().any(|context| {
                entries(&defaults(), *context, spec.title)
                    .iter()
                    .any(|entry| entry.id == spec.id)
            });
            assert!(reachable, "`{}` cannot be found in the palette", spec.id);
        }
    }

    #[test]
    fn the_context_filter_hides_what_does_not_apply() {
        let from_the_list = entries(&defaults(), Context::List, "send");
        assert!(
            !from_the_list
                .iter()
                .any(|entry| entry.id == CommandId::Send),
            "offering to send from the message list is a row that can only disappoint"
        );

        let from_the_composer = entries(&defaults(), Context::Composer, "send");
        assert!(
            from_the_composer
                .iter()
                .any(|entry| entry.id == CommandId::Send)
        );
    }

    #[test]
    fn a_command_is_findable_by_its_id_as_well_as_its_title() {
        let found = entries(&defaults(), Context::List, "archive_th");

        assert_eq!(
            found.first().map(|entry| entry.id),
            Some(CommandId::ArchiveThread)
        );
    }

    #[test]
    fn a_title_match_outranks_an_id_match_for_the_same_query() {
        let found = entries(&defaults(), Context::List, "archive");
        let ranks: Vec<CommandId> = found.iter().map(|entry| entry.id).collect();

        let archive = ranks.iter().position(|id| *id == CommandId::Archive);
        assert_eq!(archive, Some(0), "{ranks:?}");
    }

    #[test]
    fn rows_carry_the_binding_in_force() {
        let listed = entries(&defaults(), Context::List, "archive");
        let archive = listed
            .iter()
            .find(|entry| entry.id == CommandId::Archive)
            .expect("archive");

        assert_eq!(archive.binding.as_deref(), Some("a"));
    }

    #[test]
    fn a_rebound_command_shows_its_new_key() {
        let mut overrides = postio_config::KeyBindings::default();
        overrides
            .overrides_mut()
            .insert("archive".to_owned(), "y".to_owned());
        let keymap = Keymap::resolve(&overrides);

        let listed = entries(&keymap, Context::List, "archive");
        let archive = listed
            .iter()
            .find(|entry| entry.id == CommandId::Archive)
            .expect("archive");

        assert_eq!(
            archive.binding.as_deref(),
            Some("y"),
            "the palette reads the live keymap, not the registry default"
        );
    }

    #[test]
    fn a_query_that_matches_nothing_lists_nothing() {
        assert!(entries(&defaults(), Context::List, "zzzzz").is_empty());
    }

    #[test]
    fn the_list_is_capped() {
        let listed = entries(&defaults(), Context::List, "");

        assert!(listed.len() <= MAX_ROWS);
    }

    #[test]
    fn highlighting_marks_the_matched_characters_and_escapes_the_rest() {
        assert_eq!(highlight("Reply", &[0]), "<b>R</b>eply");
        assert_eq!(highlight("Move to…", &[]), "Move to…");
        assert_eq!(
            highlight("A & B", &[]),
            "A &amp; B",
            "a title is markup once it reaches a label"
        );
    }
}
