//! The settings panel: canvas 3f — `config.toml` *is* the settings UI.
//!
//! There is no second store and no OK/Cancel. The panel shows the real file
//! in a text view, section navigation on the left jumps to a header, and a
//! validity line along the foot replaces a dialog's buttons. Typing here and
//! typing in `$EDITOR` produce the same bytes on disk, because both write the
//! same thing: the literal text in the buffer, verbatim.
//!
//! That last point is why this is a [`gtk::TextView`] over raw text rather
//! than a form built from `postio_config::Config` — a form would have to
//! serialize back through [`postio_config::Config::to_toml_string`], which
//! reorders keys and drops comments (see that type's own doc comment: unknown
//! keys survive, but there is no promise about layout). Editing the actual
//! characters is what makes "comments and ordering survive a round trip" true
//! by construction instead of by careful reserialization.
//!
//! # Two halves
//!
//! [`Section`], [`find_section`] and [`section_at_line`] are pure functions
//! over the file's text, tested with no display. [`SettingsPanel`] is the
//! widget: it shows the live validity line (`postio_config::validate` does
//! the parsing and timing already; this module only formats the result), and
//! it writes the buffer back to disk on a short debounce after typing settles
//! — see [`write_atomically`] for why that write is a rename, not an
//! in-place write.
//!
//! # Revert
//!
//! [`SettingsPanel::revert`] writes the last configuration that loaded
//! without error back over the file, and says so on the footer line —
//! canvas 3f's "Revert file" button. [`SettingsPanel::note_known_good`] is
//! what keeps that memory honest when the edit that validated did not come
//! from this panel at all: `$EDITOR` writes the same file, through the same
//! watcher, and `crate::config`'s bridge reports every reload here, not only
//! the ones this widget's own buffer caused.
//!
//! # What this module does not do
//!
//! Launching `$EDITOR` itself is `crate::config`'s job: `CommandId::EditConfig`
//! resolves from the keymap (see `crates/postio-gtk/tests/gtk_live_config.rs`)
//! independently of this panel being open, and the palette already gives it
//! an accessible control, so this panel does not duplicate that with a
//! second button of its own. `CommandId::Settings` (`window.rs::run()`) is
//! what makes the panel itself reachable from a binding and the palette,
//! alongside the main menu.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;

/// How long to let typing settle before writing the buffer back to disk.
///
/// Long enough that a fast typist is not racing the disk on every keystroke;
/// short enough that "applied live" still reads as true. The file watcher's
/// own debounce (`postio_config::watch::DEFAULT_DEBOUNCE`, 120ms) runs after
/// this one settles, so the whole round trip — keystroke to reload — is well
/// under half a second.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(250);

/// How wide the section nav is, from canvas 3f.
pub const NAV_WIDTH: i32 = 150;

/// How tall the body (nav plus text) is, from canvas 3f.
pub const BODY_HEIGHT: i32 = 330;

/// What the settings panel calls the file it is showing, for a screen
/// reader. One constant because two widgets announce it — the text view and
/// the scroll region around it, which is a tab stop of its own — and a
/// region that disagrees with its content about what it holds is worse than
/// one that repeats it.
const FILE_NAME: &str = "config.toml";

// ---------------------------------------------------------------------------
// Sections — pure, no GTK
// ---------------------------------------------------------------------------

/// One of the five sections the nav lists, in canvas order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    /// `[ui]` — density, theme, hover actions, thread drill-in.
    Ui,
    /// `[keys]` — command id to binding.
    Keys,
    /// `[accounts]` — one table per account.
    Accounts,
    /// `[sync]` — IDLE, polling, connection budget.
    Sync,
    /// `[filters]` — named saved queries.
    Filters,
}

impl Section {
    /// Every section, in nav order.
    pub const ALL: [Section; 5] = [
        Section::Ui,
        Section::Keys,
        Section::Accounts,
        Section::Sync,
        Section::Filters,
    ];

    /// The top-level TOML key this section's headers start with.
    ///
    /// `[accounts]` and `[filters]` never appear as a bare header — every
    /// account and filter is its own dotted table, `[accounts.personal]` —
    /// so matching is by prefix, not by literal line.
    fn key(self) -> &'static str {
        match self {
            Section::Ui => "ui",
            Section::Keys => "keys",
            Section::Accounts => "accounts",
            Section::Sync => "sync",
            Section::Filters => "filters",
        }
    }

    /// The nav label, exactly as canvas 3f draws it.
    pub fn label(self) -> &'static str {
        match self {
            Section::Ui => "[ui]",
            Section::Keys => "[keys]",
            Section::Accounts => "[accounts]",
            Section::Sync => "[sync]",
            Section::Filters => "[filters]",
        }
    }
}

/// The first path segment of a `[...]` header line, if `line` is one.
///
/// `[accounts.personal.imap]` yields `Some("accounts")`; `density = "x"` and
/// `[[array_of_tables]]` (unused by this schema, guarded anyway) yield `None`.
fn header_key(line: &str) -> Option<&str> {
    let line = line.trim();
    let inner = line.strip_prefix('[')?.strip_suffix(']')?;
    if inner.starts_with('[') || inner.is_empty() {
        return None;
    }
    inner.split('.').next()
}

/// Every section header in `text`, as a zero-based line number, top to
/// bottom.
fn header_lines(text: &str) -> impl Iterator<Item = (usize, Section)> + '_ {
    text.lines().enumerate().filter_map(|(line, content)| {
        let key = header_key(content)?;
        Section::ALL
            .into_iter()
            .find(|section| section.key() == key)
            .map(|section| (line, section))
    })
}

/// The first line `section` is written at, if it appears in `text` at all.
pub fn find_section(text: &str, section: Section) -> Option<usize> {
    header_lines(text)
        .find(|(_, found)| *found == section)
        .map(|(line, _)| line)
}

/// Which section a cursor on `cursor_line` sits inside — the nearest header
/// at or above it. `None` above the first header, or in a file with none.
pub fn section_at_line(text: &str, cursor_line: usize) -> Option<Section> {
    header_lines(text)
        .filter(|(line, _)| *line <= cursor_line)
        .last()
        .map(|(_, section)| section)
}

// ---------------------------------------------------------------------------
// Path display — pure, no GTK
// ---------------------------------------------------------------------------

/// `path`, with the user's home directory collapsed to `~` — what the header
/// shows, matching canvas 3f's `~/.config/postmark/config.toml`.
fn display_path(path: &Path) -> String {
    std::env::var_os("HOME")
        .and_then(|home| {
            path.strip_prefix(home)
                .ok()
                .map(|rest| format!("~/{}", rest.display()))
        })
        .unwrap_or_else(|| path.display().to_string())
}

// ---------------------------------------------------------------------------
// Writing back
// ---------------------------------------------------------------------------

/// Writes `text` to `path` the way an editor saves: a temporary file in the
/// same directory, then a rename over the target.
///
/// `postio_config::watch::ConfigWatcher` is built to see exactly this shape of
/// save — a create-or-modify of a scratch file followed by a rename — rather
/// than an in-place write (see that module's docs on why: "editors replace
/// the file, they do not write it"). Saving the same way means the panel's own
/// edits reach the watcher, and therefore the rest of the running app, the
/// same way `$EDITOR`'s do.
fn write_atomically(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config.toml");
    let tmp = path.with_file_name(format!(".{name}.tmp"));
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// The widget
// ---------------------------------------------------------------------------

mod imp {
    use super::*;

    pub struct SettingsPanel {
        pub path_label: gtk::Label,
        pub tag: gtk::Label,
        pub nav: gtk::ListBox,
        pub buffer: gtk::TextBuffer,
        pub view: gtk::TextView,
        pub status: gtk::Label,
        pub revert: gtk::Button,
        pub path: RefCell<Option<PathBuf>>,
        /// Set while [`super::SettingsPanel::load`] is replacing the buffer's
        /// text, so that reload does not read back as an edit and schedule a
        /// pointless write of the very bytes just read.
        pub loading: Cell<bool>,
        pub write_source: RefCell<Option<glib::SourceId>>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
        /// The last text that loaded without error — from this panel's own
        /// typing, or from anywhere else that writes the same file. `None`
        /// only before anything has ever validated, which in practice means
        /// never: even a missing file validates to defaults.
        pub last_good: RefCell<Option<String>>,
    }

    impl Default for SettingsPanel {
        fn default() -> Self {
            Self {
                path_label: gtk::Label::new(None),
                tag: gtk::Label::new(None),
                nav: gtk::ListBox::new(),
                buffer: gtk::TextBuffer::new(None),
                view: gtk::TextView::new(),
                status: gtk::Label::new(None),
                revert: gtk::Button::with_label("Revert file"),
                path: RefCell::new(None),
                loading: Cell::new(false),
                write_source: RefCell::new(None),
                dismissed: RefCell::new(Vec::new()),
                last_good: RefCell::new(None),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SettingsPanel {
        const NAME: &'static str = "PostioSettingsPanel";
        type Type = super::SettingsPanel;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for SettingsPanel {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(source) = self.write_source.borrow_mut().take() {
                source.remove();
            }
        }
    }

    impl WidgetImpl for SettingsPanel {}
    impl BinImpl for SettingsPanel {}
}

glib::wrapper! {
    /// Canvas 3f: the settings panel over `config.toml` itself.
    pub struct SettingsPanel(ObjectSubclass<imp::SettingsPanel>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for SettingsPanel {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl SettingsPanel {
    /// An empty panel, not yet pointed at a file.
    pub fn new() -> Self {
        Self::default()
    }

    /// Reads `path` and shows it.
    ///
    /// A missing file opens empty rather than erroring: first run has nothing
    /// on disk yet, and typing here is what creates it, exactly as a first
    /// `$EDITOR` save would. A file that is already usable seeds
    /// [`SettingsPanel::revert`]'s target, the same as any later save —
    /// otherwise a config nobody has edited since startup would have nothing
    /// to revert to.
    pub fn load(&self, path: &Path) {
        let imp = self.imp();
        *imp.path.borrow_mut() = Some(path.to_path_buf());
        imp.path_label.set_label(&display_path(path));

        let text = std::fs::read_to_string(path).unwrap_or_default();
        imp.loading.set(true);
        imp.buffer.set_text(&text);
        imp.loading.set(false);

        self.refresh_validity();
        if postio_config::validate::check_str(&text)
            .validation
            .is_valid()
        {
            self.note_known_good(&text);
        }
    }

    /// The file this panel is showing, if [`SettingsPanel::load`] has been
    /// called.
    pub fn path(&self) -> Option<PathBuf> {
        self.imp().path.borrow().clone()
    }

    /// The buffer's full text, exactly as typed.
    pub fn text(&self) -> String {
        let buffer = &self.imp().buffer;
        let (start, end) = buffer.bounds();
        buffer.text(&start, &end, true).to_string()
    }

    /// Replaces the buffer's text, as though the user had typed it: the
    /// validity line recomputes and a write is scheduled, same as any other
    /// edit. A test seam, in the same spirit as `crate::palette::Palette`'s
    /// `set_query`.
    pub fn set_text(&self, text: &str) {
        self.imp().buffer.set_text(text);
    }

    /// The footer line exactly as shown: the validity line, or — after
    /// [`SettingsPanel::revert`] — the confirmation that replaces it until
    /// the next edit. A test seam, in the same spirit as [`SettingsPanel::set_text`].
    pub fn footer_text(&self) -> String {
        self.imp().status.label().to_string()
    }

    /// Whether the buffer's current text is usable as written — what the
    /// validity tag shows.
    pub fn is_valid(&self) -> bool {
        postio_config::validate::check_str(&self.text())
            .validation
            .is_valid()
    }

    /// Called when the user presses `Escape`.
    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.imp().dismissed.borrow_mut().push(Box::new(handler));
    }

    fn dismiss(&self) {
        for handler in self.imp().dismissed.borrow().iter() {
            handler();
        }
    }

    /// Scrolls to and places the cursor at `section`'s header.
    ///
    /// When the section is not written yet, jumps to the end of the file
    /// instead — a reasonable place to start typing it in.
    fn jump_to(&self, section: Section) {
        let imp = self.imp();
        let text = self.text();
        let mut iter = match find_section(&text, section).and_then(|line| {
            imp.buffer
                .iter_at_line(i32::try_from(line).unwrap_or(i32::MAX))
        }) {
            Some(iter) => iter,
            None => imp.buffer.end_iter(),
        };
        imp.buffer.place_cursor(&iter);
        imp.view.scroll_to_iter(&mut iter, 0.0, false, 0.0, 0.0);
        imp.view.grab_focus();
    }

    /// Recomputes the validity line from the buffer's current text.
    ///
    /// `postio_config::validate` does the actual parsing and timing; this
    /// only formats what it reports into the tag and the foot line canvas 3f
    /// draws. Deliberately does not feed [`SettingsPanel::revert`]'s target:
    /// `gtk::TextBuffer::set_text` fires `changed` once for the delete and
    /// once for the insert, so mid-edit this sees a transiently empty buffer
    /// — and an empty file is valid TOML. Recording that as "last good" would
    /// make every edit briefly overwrite the real one. [`note_known_good`]
    /// only ever sees text that actually reached disk, which a buffer signal
    /// firing mid-mutation cannot promise.
    ///
    /// [`note_known_good`]: SettingsPanel::note_known_good
    fn refresh_validity(&self) {
        let imp = self.imp();
        let text = self.text();
        let checked = postio_config::validate::check_str(&text);
        let valid = checked.validation.is_valid();

        imp.tag.set_label(if valid { "valid" } else { "invalid" });
        if valid {
            imp.tag.remove_css_class("invalid");
        } else {
            imp.tag.add_css_class("invalid");
        }

        imp.status.set_label(&format!(
            "{} · applied live · nothing to save",
            checked.validation.status_line()
        ));
    }

    /// Records `text` as the last configuration known to load without error,
    /// without touching the buffer.
    ///
    /// For a save that reached the file some way other than typing here —
    /// `$EDITOR`, most of all. Keeping this separate from the buffer-driven
    /// validity check is what stops a `$EDITOR` save from clobbering an edit
    /// in progress in this panel: it updates what "last good" means without
    /// ever touching what is on screen.
    pub fn note_known_good(&self, text: &str) {
        *self.imp().last_good.borrow_mut() = Some(text.to_string());
    }

    /// Restores the file to the last configuration known to load without
    /// error, and says so on the footer line.
    ///
    /// Quietly does nothing when there is no path to write to, or nothing has
    /// ever validated — the second case cannot happen in practice, since even
    /// a missing file validates to defaults, but pretending there is always
    /// something to revert to would risk overwriting a first, never-saved
    /// edit with nothing.
    pub fn revert(&self) {
        let imp = self.imp();
        let Some(text) = imp.last_good.borrow().clone() else {
            return;
        };
        let Some(path) = imp.path.borrow().clone() else {
            return;
        };
        if let Err(error) = write_atomically(&path, &text) {
            tracing::error!(path = %path.display(), %error, "cannot revert the config file");
            return;
        }
        imp.loading.set(true);
        imp.buffer.set_text(&text);
        imp.loading.set(false);
        self.refresh_validity();
        imp.status
            .set_label("Reverted to the last configuration that loaded without error.");
    }

    /// Highlights whichever nav row the cursor currently sits inside.
    fn sync_nav(&self) {
        let imp = self.imp();
        let offset = imp.buffer.cursor_position();
        let line = usize::try_from(imp.buffer.iter_at_offset(offset).line()).unwrap_or(0);

        match section_at_line(&self.text(), line) {
            Some(section) => {
                let index = Section::ALL
                    .iter()
                    .position(|candidate| *candidate == section)
                    .unwrap_or(0);
                if let Some(row) = imp.nav.row_at_index(index as i32) {
                    imp.nav.select_row(Some(&row));
                }
            }
            None => imp.nav.select_row(None::<&gtk::ListBoxRow>),
        }
    }

    /// Writes the buffer to disk after [`WRITE_DEBOUNCE`] of quiet typing.
    fn schedule_write(&self) {
        let imp = self.imp();
        if let Some(source) = imp.write_source.borrow_mut().take() {
            source.remove();
        }
        let weak = self.downgrade();
        let id = glib::timeout_add_local(WRITE_DEBOUNCE, move || {
            if let Some(panel) = weak.upgrade() {
                panel.imp().write_source.replace(None);
                panel.write_now();
            }
            glib::ControlFlow::Break
        });
        *imp.write_source.borrow_mut() = Some(id);
    }

    fn write_now(&self) {
        let Some(path) = self.imp().path.borrow().clone() else {
            return;
        };
        if let Err(error) = write_atomically(&path, &self.text()) {
            tracing::error!(path = %path.display(), %error, "cannot save the config file");
        }
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-settings");
        self.set_halign(gtk::Align::Center);
        self.set_valign(gtk::Align::Center);

        // A dialog to a screen reader: it takes the keyboard and `Escape`
        // closes it, the same contract as the cheat sheet.
        self.set_accessible_role(gtk::AccessibleRole::Dialog);
        self.update_property(&[gtk::accessible::Property::Label("Settings")]);

        // ── header: file path, validity tag ──────────────────────────────
        imp.path_label.set_xalign(0.0);
        imp.path_label.add_css_class("postio-settings-path");
        imp.tag.add_css_class("postio-settings-tag");

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("postio-settings-header");
        header.append(&imp.path_label);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        header.append(&spacer);
        header.append(&imp.tag);

        // ── body: section nav, the file itself ───────────────────────────
        imp.nav.add_css_class("postio-settings-nav-list");
        imp.nav.set_selection_mode(gtk::SelectionMode::Single);
        imp.nav.set_activate_on_single_click(true);
        for section in Section::ALL {
            let row = gtk::ListBoxRow::new();
            row.add_css_class("postio-settings-nav-row");
            let label = gtk::Label::new(Some(section.label()));
            label.set_xalign(0.0);
            row.set_child(Some(&label));
            row.update_property(&[gtk::accessible::Property::Label(section.label())]);
            imp.nav.append(&row);
        }
        imp.nav.connect_row_activated(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                if let Some(section) = Section::ALL.get(row.index() as usize) {
                    panel.jump_to(*section);
                }
            }
        ));

        let nav_scroller = gtk::ScrolledWindow::new();
        nav_scroller.set_child(Some(&imp.nav));
        nav_scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        nav_scroller.add_css_class("postio-settings-nav");
        nav_scroller.set_size_request(NAV_WIDTH, -1);
        // A scroll area takes the keyboard so it can be scrolled with one,
        // which means Tab stops here and a screen reader has to have
        // something to say. The rows inside are named individually; this
        // names the region they sit in.
        nav_scroller.update_property(&[gtk::accessible::Property::Label("Settings sections")]);

        imp.view.set_buffer(Some(&imp.buffer));
        imp.view.set_monospace(true);
        imp.view.set_wrap_mode(gtk::WrapMode::WordChar);
        imp.view.set_top_margin(4);
        imp.view.set_left_margin(4);
        imp.view.add_css_class("postio-settings-view");
        imp.view
            .update_property(&[gtk::accessible::Property::Label(FILE_NAME)]);

        let view_scroller = gtk::ScrolledWindow::new();
        view_scroller.set_child(Some(&imp.view));
        view_scroller.set_hexpand(true);
        view_scroller.set_vexpand(true);
        // A scroll area takes the keyboard so it can be scrolled with one, so
        // Tab stops here before it reaches the text and a screen reader needs
        // something to say at that stop.
        view_scroller.update_property(&[gtk::accessible::Property::Label(FILE_NAME)]);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("postio-settings-body");
        body.append(&nav_scroller);
        body.append(&gtk::Separator::new(gtk::Orientation::Vertical));
        body.append(&view_scroller);
        body.set_size_request(-1, BODY_HEIGHT);

        // ── footer: the validity line, and Revert file ───────────────────
        imp.status.set_xalign(0.0);
        imp.status.set_hexpand(true);
        imp.status.add_css_class("postio-settings-footer");
        imp.revert.add_css_class("postio-settings-revert");
        imp.revert.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.revert()
        ));

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        footer.add_css_class("postio-settings-footer-row");
        footer.append(&imp.status);
        footer.append(&imp.revert);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        column.append(&body);
        column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        column.append(&footer);
        self.set_child(Some(&column));

        imp.buffer.connect_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| {
                if panel.imp().loading.get() {
                    return;
                }
                panel.refresh_validity();
                panel.schedule_write();
            }
        ));
        imp.buffer.connect_cursor_position_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.sync_nav()
        ));

        // `Escape` closes it, caught in the capture phase so it works
        // regardless of which child — the nav list or the text view — has
        // focus. The same contract `window.rs`'s own controller uses.
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        keys.connect_key_pressed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, key, _, _| {
                if key == gtk::gdk::Key::Escape {
                    panel.dismiss();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
        ));
        self.add_controller(keys);

        self.refresh_validity();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
# edits here and in the panel are the same file
[ui]
density = \"compact\"
theme = \"system\"

[keys]
archive = \"a\"

[accounts.personal]
email = \"ada@example.com\"

[accounts.personal.imap]
host = \"imap.example.com\"

[sync]
idle = true
";

    // -- header parsing -----------------------------------------------------

    #[test]
    fn a_bare_section_header_names_its_key() {
        assert_eq!(header_key("[ui]"), Some("ui"));
        assert_eq!(header_key("  [sync]  "), Some("sync"));
    }

    #[test]
    fn a_dotted_header_is_matched_by_its_first_segment() {
        assert_eq!(header_key("[accounts.personal.imap]"), Some("accounts"));
        assert_eq!(header_key("[filters.urgent]"), Some("filters"));
    }

    #[test]
    fn a_key_value_line_has_no_header() {
        assert_eq!(header_key("density = \"compact\""), None);
        assert_eq!(header_key(""), None);
    }

    #[test]
    fn an_array_of_tables_header_is_not_mistaken_for_a_section() {
        // Unused by this schema today; guarded so a future one does not
        // silently misfile under the wrong section.
        assert_eq!(header_key("[[items]]"), None);
    }

    // -- find_section ---------------------------------------------------

    #[test]
    fn find_section_locates_a_bare_header() {
        assert_eq!(find_section(SAMPLE, Section::Ui), Some(1));
        assert_eq!(find_section(SAMPLE, Section::Sync), Some(14));
    }

    #[test]
    fn find_section_locates_the_first_dotted_table_for_a_prefix_section() {
        // `[accounts]` never appears bare; the first `[accounts.*]` table is
        // where a click on "[accounts]" has to land.
        assert_eq!(find_section(SAMPLE, Section::Accounts), Some(8));
    }

    #[test]
    fn find_section_is_none_for_a_section_not_written_yet() {
        assert_eq!(find_section(SAMPLE, Section::Filters), None);
    }

    // -- section_at_line --------------------------------------------------

    #[test]
    fn section_at_line_is_none_above_the_first_header() {
        assert_eq!(section_at_line(SAMPLE, 0), None);
    }

    #[test]
    fn section_at_line_finds_the_nearest_header_at_or_above() {
        assert_eq!(section_at_line(SAMPLE, 2), Some(Section::Ui));
        assert_eq!(section_at_line(SAMPLE, 3), Some(Section::Ui));
        assert_eq!(section_at_line(SAMPLE, 6), Some(Section::Keys));
    }

    #[test]
    fn section_at_line_treats_every_line_of_a_dotted_table_as_its_section() {
        // Line 11, `host = "imap.example.com"`, sits under
        // `[accounts.personal.imap]`, which is still `accounts`.
        assert_eq!(section_at_line(SAMPLE, 11), Some(Section::Accounts));
    }

    #[test]
    fn section_at_line_past_the_end_of_the_file_is_the_last_section() {
        assert_eq!(section_at_line(SAMPLE, 999), Some(Section::Sync));
    }

    // -- display_path -------------------------------------------------------

    #[test]
    fn a_path_under_home_is_shown_with_a_tilde() {
        // SAFETY: single-threaded test; nothing else reads `HOME` here.
        unsafe { std::env::set_var("HOME", "/home/example") };
        assert_eq!(
            display_path(Path::new("/home/example/.config/postio/config.toml")),
            "~/.config/postio/config.toml"
        );
    }

    #[test]
    fn a_path_outside_home_is_shown_verbatim() {
        // SAFETY: single-threaded test; nothing else reads `HOME` here.
        unsafe { std::env::set_var("HOME", "/home/example") };
        assert_eq!(
            display_path(Path::new("/etc/postio/config.toml")),
            "/etc/postio/config.toml"
        );
    }

    // -- write_atomically -----------------------------------------------

    #[test]
    fn write_atomically_replaces_the_file_and_cleans_up_the_scratch_file() {
        let dir = std::env::temp_dir().join(format!(
            "postio-settings-write-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");

        write_atomically(&path, "[ui]\ndensity = \"compact\"\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[ui]\ndensity = \"compact\"\n"
        );
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .filter(|entry| entry.file_name() != "config.toml")
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");

        write_atomically(&path, "[ui]\ndensity = \"comfortable\"\n").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[ui]\ndensity = \"comfortable\"\n"
        );

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_atomically_creates_missing_parent_directories() {
        let dir = std::env::temp_dir().join(format!(
            "postio-settings-write-parent-{}-{}",
            std::process::id(),
            line!()
        ));
        let path = dir.join("nested").join("config.toml");

        write_atomically(&path, "[ui]\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[ui]\n");

        std::fs::remove_dir_all(&dir).ok();
    }
}
