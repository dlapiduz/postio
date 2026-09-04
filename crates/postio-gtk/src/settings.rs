//! The settings panel: canvas 3f — `config.toml` *is* the settings UI.
//!
//! There is no second store and no OK/Cancel. The panel shows the real file
//! — a [`gtk::TextView`] over its raw text, the only pane left that still
//! works that way (see below) — section navigation on the left jumps to a
//! header, and a validity line along the foot replaces a dialog's buttons.
//! Typing here and typing in `$EDITOR` produce the same bytes on disk,
//! because both write the same thing: the literal text in the buffer,
//! verbatim.
//!
//! # Structured panes patch, they never reserialize
//!
//! [`Section::Filters`] and [`Section::Ui`] are *forms* over the same file —
//! not the raw-text exception the rest of this doc describes. Building one
//! by serializing a whole `postio_config::Config` back through
//! [`postio_config::Config::to_toml_string`] would reorder every key and drop
//! every comment in the file, not only in the one table the pane owns (see
//! that method's own doc comment: unknown keys survive, but there is no
//! promise about layout) — which is exactly the trap a raw-text view avoids
//! by construction and a naive form would fall straight into. So a structured
//! pane never reserializes: [`patch_filters`] and [`patch_ui`] rewrite only
//! their own table with `toml_edit`'s format-preserving document model
//! ([`SettingsPanel::apply_filters_mutation`],
//! [`SettingsPanel::apply_ui_mutation`]), and the result is written into
//! *this* buffer, so it reaches disk through the exact same debounced write
//! every raw edit already does. `[keys]` and `[filters]`'s advanced escape
//! hatch stay on the raw view below until their own issues convert them the
//! same way.
//!
//! [`Section::Privacy`] is a third, stranger kind: not a form over a table,
//! because there is no table — the remote-image allow-list it manages lives
//! entirely outside `config.toml` (see [`Section::key`]'s own doc). It reads
//! and writes [`crate::reader::RemoteImageAllowList`] directly, with nothing
//! for the debounced buffer write to do.
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

use std::cell::{Cell, OnceCell, RefCell};
use std::path::{Path, PathBuf};
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_config::filters::{FilterConfig, Reorder};
use postio_config::sync::{AttachmentFetch, CheckForMail};
use postio_config::{
    Config, Density, SyncConfig, Theme, patch_filters, patch_keys, patch_sync, patch_ui,
};
use postio_core::CommandId;
use postio_model::ids::SignatureId;
use postio_model::{Account, AccountId, UnsubscribeActivation};

use crate::keymap::{Chord, ChordFromGdk};

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

/// How tall the accounts list grows before it scrolls (#464).
const ACCOUNTS_MAX_HEIGHT: i32 = 160;

/// What an account row's context menu asked for (#464, ADR 0005 Q6a).
///
/// Not itself a `CommandId`: both need a specific account as their payload,
/// which a keystroke carries no default for. `CommandId::RemoveAccount` and
/// `CommandId::UpdateCredential` (#471) reach the keyboard path by resolving
/// [`SettingsPanel::focused_account`] and calling
/// [`SettingsPanel::request_account_action`] with the same variant the
/// context menu would have -- one payload type either entry point ends in,
/// rather than a second one the registry would have to know about.
/// `SavedSearchAction` (#292) is the same shape for the same reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAction {
    /// Open the reauthenticate screen for this account.
    UpdateCredential,
    /// Mark the account for removal.
    Remove,
}

/// What to call when an account row's context menu picks an action.
type AccountActionHandler = Box<dyn Fn(AccountId, AccountAction)>;

/// What to call when an account row's enabled switch is flipped by hand —
/// never fired for the initial state [`SettingsPanel::set_accounts`] sets.
type AccountEnabledHandler = Box<dyn Fn(AccountId, bool)>;

/// One field of the account detail view (#880) committed to a new value.
///
/// An account is database state, not `config.toml` preference (ADR 0005
/// Q6b), so this panel cannot patch a buffer the way [`Section::Filters`]
/// and [`Section::Ui`] do — it only reports what changed, the same split
/// [`AccountAction`] already uses, and `postio-app`'s `settings_accounts`
/// module is what actually calls `AccountRepository::update`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountEdit {
    /// The name shown in the sidebar and this row.
    DisplayName(String),
    /// The IMAP server's hostname.
    ImapHost(String),
    /// The IMAP server's port.
    ImapPort(u16),
    /// The SMTP server's hostname.
    SmtpHost(String),
    /// The SMTP server's port.
    SmtpPort(u16),
    /// Which of the account's signatures the composer starts on (#979).
    ///
    /// `Option` because "none of them" is a real answer the model already
    /// holds — `Account::default_signature_id` is an `Option<SignatureId>`,
    /// and an account can have signatures without preferring one.
    DefaultSignature(Option<SignatureId>),
}

/// What to call when a field in the account detail view is committed.
type AccountEditHandler = Box<dyn Fn(AccountId, AccountEdit)>;

/// Who to tell when somebody asks whether an account's settings work (#980).
type TestConnectionHandler = Box<dyn Fn(AccountId)>;

/// What the detail view has to say about the last connection test.
///
/// Three states and no fourth: #980's acceptance is "a visible result:
/// success, or a real error message, not a spinner that silently stops", and
/// a type that can only be these cannot express the spinner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    /// Nothing has been asked yet, so there is nothing to say.
    Idle,
    /// A test is running.
    Testing,
    /// It finished. Each server answered for itself — `Err` carries what that
    /// server or its transport said, because "it does not work" sends
    /// somebody to two screens of settings with nothing to go on.
    Answered {
        /// The incoming (IMAP) server.
        incoming: Result<(), String>,
        /// The outgoing (SMTP) server.
        outgoing: Result<(), String>,
    },
}

impl ConnectionStatus {
    /// The sentence the detail view shows, and the one a screen reader is
    /// given. Empty only for [`Idle`](ConnectionStatus::Idle), which draws
    /// nothing at all rather than an empty row.
    pub fn message(&self) -> String {
        match self {
            ConnectionStatus::Idle => String::new(),
            ConnectionStatus::Testing => "Testing…".to_owned(),
            ConnectionStatus::Answered { incoming, outgoing } => match (incoming, outgoing) {
                (Ok(()), Ok(())) => "Both servers answered.".to_owned(),
                (Err(reason), Ok(())) => format!("Incoming: {reason}"),
                (Ok(()), Err(reason)) => format!("Outgoing: {reason}"),
                (Err(incoming), Err(outgoing)) => {
                    format!("Incoming: {incoming}\nOutgoing: {outgoing}")
                }
            },
        }
    }

    /// Whether this is a state the user should read as a problem, so the row
    /// can carry the failure styling rather than the widget guessing from the
    /// text.
    fn failed(&self) -> bool {
        matches!(
            self,
            ConnectionStatus::Answered { incoming, outgoing }
                if incoming.is_err() || outgoing.is_err()
        )
    }
}

// ---------------------------------------------------------------------------
// Sections — pure, no GTK
// ---------------------------------------------------------------------------

/// One of the six sections the nav lists, in canvas order.
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
    /// The remote-image allow-list (#871) — never a `config.toml` table at
    /// all, unlike every other section here: it is view state, kept in its
    /// own `$XDG_STATE_HOME` key-file (see
    /// [`crate::reader::RemoteImageAllowList`]'s own module doc for why).
    Privacy,
}

impl Section {
    /// Every section, in nav order.
    pub const ALL: [Section; 6] = [
        Section::Ui,
        Section::Keys,
        Section::Accounts,
        Section::Sync,
        Section::Filters,
        Section::Privacy,
    ];

    /// The top-level TOML key this section's headers start with.
    ///
    /// `[accounts]` and `[filters]` never appear as a bare header — every
    /// account and filter is its own dotted table, `[accounts.personal]` —
    /// so matching is by prefix, not by literal line. `Privacy` never
    /// appears at all, the same as `Accounts` since #470: the nav item
    /// stays and points at a structured widget instead of any text.
    fn key(self) -> &'static str {
        match self {
            Section::Ui => "ui",
            Section::Keys => "keys",
            Section::Accounts => "accounts",
            Section::Sync => "sync",
            Section::Filters => "filters",
            Section::Privacy => "privacy",
        }
    }

    /// The nav label. `Privacy` deliberately drops the `[table]` bracket
    /// style the others use — see [`key`](Self::key): it would claim a
    /// `config.toml` table this section has never had.
    pub fn label(self) -> &'static str {
        match self {
            Section::Ui => "[ui]",
            Section::Keys => "[keys]",
            Section::Accounts => "[accounts]",
            Section::Sync => "[sync]",
            Section::Filters => "[filters]",
            Section::Privacy => "Privacy",
        }
    }
}

/// Every saved search's key, in the order the structured filters pane shows
/// them: pinned ones first, in their sidebar order
/// ([`Config::ordered_filter_keys`]), then anything unpinned — the sidebar
/// never shows those, so this settings pane is the only place they are
/// visible at all, and alphabetical by key is the same fallback
/// `ordered_filter_keys` already gives pinned filters with no explicit
/// order.
fn filter_display_order(config: &Config) -> Vec<String> {
    let mut keys = config.ordered_filter_keys();
    let mut unpinned: Vec<&String> = config
        .filters
        .keys()
        .filter(|key| !keys.iter().any(|pinned| pinned == *key))
        .collect();
    unpinned.sort();
    keys.extend(unpinned.into_iter().cloned());
    keys
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
// Account row data (#464)
// ---------------------------------------------------------------------------

/// The account id `SettingsPanel::account_row` stamped onto `row`, or
/// [`AccountId::UNASSIGNED`] if this is not an account row at all.
fn row_account_id(row: &gtk::ListBoxRow) -> AccountId {
    // glib cannot know the type a key was stored under; this file can — see
    // `account_row`'s own comment.
    #[allow(unsafe_code)]
    unsafe {
        row.data::<i64>("postio-account-id")
            .map(|p| AccountId::new(*p.as_ref()))
            .unwrap_or(AccountId::UNASSIGNED)
    }
}

/// An account row's connection-type and auth-method badge — "IMAP ·
/// password", "Gmail · OAuth 2" — both already on the account itself, so
/// unlike the mail weight and the token validity this needs nothing handed
/// in from the composition root (#878).
fn account_badge(account: &Account) -> String {
    let backend = match &account.backend {
        postio_model::account::Backend::Imap => "IMAP",
        postio_model::account::Backend::Jmap { .. } => "JMAP",
        postio_model::account::Backend::Gmail => "Gmail",
    };
    let auth = match account.auth {
        postio_model::account::AuthMethod::Password => "password",
        postio_model::account::AuthMethod::AppPassword => "app password",
        postio_model::account::AuthMethod::OAuth2 => "OAuth 2",
        postio_model::account::AuthMethod::XOAuth2 => "OAuth 2",
    };
    format!("{backend} · {auth}")
}

/// One labeled field in the account detail view (#880) — a plain label over
/// the control, the simplest shape that fits an `Entry` or a `SpinButton`
/// equally well. Unlike [`SettingsPanel::sync_row`], there is no second
/// description line: a host or a port names itself.
fn detail_row(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    let title = gtk::Label::new(Some(label));
    title.set_xalign(0.0);
    title.add_css_class("postio-settings-account-detail-label");

    let row = gtk::Box::new(gtk::Orientation::Vertical, 2);
    row.add_css_class("postio-settings-account-detail-row");
    row.append(&title);
    row.append(control);
    row
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
        /// What the key resolver could not make sense of, as of the last time
        /// [`super::SettingsPanel::set_keymap_problems`] was told. `window.rs`
        /// computes these -- they need the full command registry, which this
        /// module has no reason to depend on -- and hands them over so they
        /// render where `[keys]` is actually edited rather than only in a log
        /// line nobody watches interactively.
        pub keymap_problems: RefCell<Vec<String>>,
        /// One row per account, enable switch and context menu (#464).
        pub accounts_list: gtk::ListBox,
        /// The egress log's audit list (#151): what left this machine.
        pub egress_list: gtk::ListBox,
        pub egress_scroller: gtk::ScrolledWindow,
        /// Hidden entirely when there are no accounts to show a row for.
        pub accounts_scroller: gtk::ScrolledWindow,
        /// The accounts the rows above were built from — kept so a right
        /// click can find which id and which context-menu items (first/last
        /// have nothing to say) belong to the row it landed on.
        pub accounts: RefCell<Vec<Account>>,
        /// What each account's mail weighs, and whether the current
        /// `[sync] attachment_fetch` is already pulling payloads.
        ///
        /// Kept apart from `accounts` because the two arrive from different
        /// places -- the account list from the accounts table, the footprints
        /// from a per-account measurement -- and neither call may assume it
        /// runs after the other (#411).
        pub weights: RefCell<Vec<(AccountId, postio_core::event::MailFootprint)>>,
        pub attachments_included: std::cell::Cell<bool>,
        /// Every OAuth account's persisted token expiry, as of the last read
        /// (#878, on top of #870's persistence). An id present with `None`
        /// is a real answer — a provider that has a token on file but never
        /// said `expires_in`, so there is nothing to count down. An id
        /// simply absent is a different fact: a password account, or an
        /// OAuth account fed by an external broker (which never persists
        /// an expiry at all), so the row shows no validity line rather
        /// than a wrong one.
        pub token_expiries: RefCell<Vec<(AccountId, Option<std::time::SystemTime>)>>,
        /// The account row context menu currently open, if one is — tracked
        /// so a second right click closes the first, the same reason
        /// `Sidebar` tracks `saved_search_menu`.
        pub account_menu: RefCell<Option<gtk::PopoverMenu>>,
        pub account_action: RefCell<Vec<AccountActionHandler>>,
        pub account_enabled_changed: RefCell<Vec<AccountEnabledHandler>>,
        /// The account detail view (#880): editable display name, IMAP and
        /// SMTP host/port, over an account's real settings. Hidden until
        /// [`super::SettingsPanel::open_account_detail`] is called.
        pub account_detail: gtk::Box,
        /// Which account the detail view is currently open on, if any —
        /// what an edit's committed value is reported against.
        pub account_detail_id: RefCell<Option<AccountId>>,
        /// Built on first use by
        /// [`super::SettingsPanel::ensure_account_detail_fields`], never in
        /// `build()` or this struct's own `Default` — see that method's
        /// doc for why.
        pub account_detail_display_name: OnceCell<gtk::Entry>,
        pub account_detail_imap_host: OnceCell<gtk::Entry>,
        pub account_detail_imap_port: OnceCell<gtk::SpinButton>,
        pub account_detail_smtp_host: OnceCell<gtk::Entry>,
        pub account_detail_smtp_port: OnceCell<gtk::SpinButton>,
        /// #979's picker, and the ids behind its rows — the widget carries
        /// names because that is what a person picks by, and the handler
        /// needs the id the row stands for.
        pub account_detail_signature: OnceCell<gtk::DropDown>,
        /// The whole row, label included: hiding only the control would
        /// leave a "Default signature" label with nothing beside it.
        pub account_detail_signature_row: OnceCell<gtk::Box>,
        pub account_detail_signature_ids: RefCell<Vec<SignatureId>>,
        /// Set while [`super::SettingsPanel::open_account_detail`] is
        /// populating the fields above, so setting an `Entry`'s text does
        /// not itself fire an edit — the same guard [`SettingsPanel::load`]
        /// uses on the raw buffer, for the same reason.
        pub account_detail_loading: Cell<bool>,
        pub account_edited: RefCell<Vec<AccountEditHandler>>,
        /// Who to tell when "Test connection" is pressed (#980). The panel
        /// never dials anything itself, exactly as it never writes an edit
        /// itself: `postio-app` owns the store and the network.
        pub test_connection: RefCell<Vec<TestConnectionHandler>>,
        /// The control and the line under it, built with the rest of the
        /// detail fields and then only ever relabelled.
        pub account_detail_test_button: OnceCell<gtk::Button>,
        pub account_detail_test_status: OnceCell<gtk::Label>,
        /// One row per saved search, pinned or not (#869) — the structured
        /// pane [`Section::Filters`] now shows instead of only jumping the
        /// raw text view to `[filters]`.
        pub filters_list: gtk::ListBox,
        pub filters_scroller: gtk::ScrolledWindow,
        /// Shown instead of `filters_scroller` when there is nothing saved
        /// yet — canvas's "empty is never blank" rule; see
        /// `SettingsPanel::redraw_filters`.
        pub filters_empty: gtk::Label,
        /// `[sync]`'s structured rows (#874) — always exactly five rows, so
        /// no empty state to draw, the same shape `[ui]`'s own pane (#873)
        /// established.
        pub sync_box: gtk::Box,
        /// `[ui]`'s structured rows (#873) — unlike filters and accounts,
        /// always exactly six rows, so no empty state to draw.
        pub ui_box: gtk::Box,
        /// One row per sender with a standing remote-image exception (#871).
        pub privacy_list: gtk::ListBox,
        /// Hidden entirely when nobody is allow-listed.
        pub privacy_scroller: gtk::ScrolledWindow,
        /// Shown instead of `privacy_scroller` when the list is empty —
        /// same "empty is never blank" rule `filters_empty` follows.
        pub privacy_empty: gtk::Label,
        /// The allow-list this panel is showing, and the path a revoke
        /// writes back to — handed in by `window.rs`'s
        /// [`super::SettingsPanel::set_remote_image_allowlist`] rather than
        /// loaded here, the same reason `Window::new_reader` takes its own
        /// path rather than hardcoding [`crate::reader::RemoteImageAllowList::path`]:
        /// a test needs a scratch path, not the real state directory.
        pub remote_image_allowlist: RefCell<Option<(crate::reader::RemoteImageAllowList, PathBuf)>>,
        /// One row per past one-click-unsubscribe activation (#971), newest
        /// first — the log itself, not something this pane can act on: it is
        /// read-only history, unlike `privacy_list`'s revocable exceptions.
        pub unsubscribe_list: gtk::ListBox,
        /// Hidden entirely when nothing has ever been activated.
        pub unsubscribe_scroller: gtk::ScrolledWindow,
        /// Shown instead of `unsubscribe_scroller` when the log is empty —
        /// same "empty is never blank" rule `privacy_empty` follows.
        pub unsubscribe_empty: gtk::Label,
        /// What `redraw_unsubscribe_activations` last drew — handed in by
        /// `window.rs`, the same reason `remote_image_allowlist` is handed
        /// in rather than read here: `postio-gtk` has no SQL of its own.
        pub unsubscribe_activations: RefCell<Vec<UnsubscribeActivation>>,
        /// How many messages have asked for a read receipt (#970) — a count,
        /// not a toggle: Postio never sends one automatically (CLAUDE.md's
        /// privacy section), so there is nothing here to switch, only a fact
        /// to state. Always visible, never hidden the way an empty list is:
        /// zero is itself the answer, not the absence of one.
        pub read_receipt_count: gtk::Label,
        /// One row per registered command (#881), always present — the same
        /// no-empty-state shape `sync_box`/`ui_box` use, since the registry
        /// is never empty. Scrolled rather than a bare list, unlike those
        /// two: the registry runs to dozens of rows, not five or six.
        pub keys_list: gtk::ListBox,
        pub keys_scroller: gtk::ScrolledWindow,
        /// Which command's row is waiting for the next keypress, if any.
        pub capturing: RefCell<Option<CommandId>>,
        /// The last capture attempt's rejection, if the most recent one was
        /// rejected — read by `key_row` so the row that was being captured
        /// keeps saying why after the redraw a rejection triggers. Cleared
        /// the next time that row's rebind button is pressed.
        pub capture_conflict: RefCell<Option<(CommandId, String)>>,
        /// Set once [`super::SettingsPanel::ensure_capture_controller`] has
        /// added `keys_list`'s `EventControllerKey` — never in `build()`,
        /// see that method's own doc for why.
        pub capture_controller_installed: Cell<bool>,
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
                keymap_problems: RefCell::new(Vec::new()),
                accounts_list: gtk::ListBox::new(),
                egress_list: gtk::ListBox::new(),
                egress_scroller: gtk::ScrolledWindow::new(),
                accounts_scroller: gtk::ScrolledWindow::new(),
                accounts: RefCell::new(Vec::new()),
                token_expiries: RefCell::new(Vec::new()),
                weights: RefCell::new(Vec::new()),
                attachments_included: Cell::new(false),
                account_menu: RefCell::new(None),
                account_action: RefCell::new(Vec::new()),
                account_enabled_changed: RefCell::new(Vec::new()),
                account_detail: gtk::Box::new(gtk::Orientation::Vertical, 8),
                account_detail_id: RefCell::new(None),
                account_detail_display_name: OnceCell::new(),
                account_detail_imap_host: OnceCell::new(),
                account_detail_imap_port: OnceCell::new(),
                account_detail_smtp_host: OnceCell::new(),
                account_detail_smtp_port: OnceCell::new(),
                account_detail_signature: OnceCell::new(),
                account_detail_signature_row: OnceCell::new(),
                account_detail_signature_ids: RefCell::new(Vec::new()),
                account_detail_loading: Cell::new(false),
                account_edited: RefCell::new(Vec::new()),
                test_connection: RefCell::new(Vec::new()),
                account_detail_test_button: OnceCell::new(),
                account_detail_test_status: OnceCell::new(),
                filters_list: gtk::ListBox::new(),
                filters_scroller: gtk::ScrolledWindow::new(),
                filters_empty: gtk::Label::new(Some(
                    "No saved searches yet — press Ctrl+S in search to save one.",
                )),
                sync_box: gtk::Box::new(gtk::Orientation::Vertical, 0),
                ui_box: gtk::Box::new(gtk::Orientation::Vertical, 0),
                privacy_list: gtk::ListBox::new(),
                privacy_scroller: gtk::ScrolledWindow::new(),
                privacy_empty: gtk::Label::new(Some(
                    "No senders are always allowed to load remote images.",
                )),
                remote_image_allowlist: RefCell::new(None),
                unsubscribe_list: gtk::ListBox::new(),
                unsubscribe_scroller: gtk::ScrolledWindow::new(),
                unsubscribe_empty: gtk::Label::new(Some("No mailing lists have been left yet.")),
                unsubscribe_activations: RefCell::new(Vec::new()),
                read_receipt_count: gtk::Label::new(None),
                keys_list: gtk::ListBox::new(),
                keys_scroller: gtk::ScrolledWindow::new(),
                capturing: RefCell::new(None),
                capture_conflict: RefCell::new(None),
                capture_controller_installed: Cell::new(false),
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
        self.redraw_filters();
        self.redraw_sync();
        self.redraw_ui();
        self.redraw_keys();
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
    ///
    /// Redraws explicitly rather than leaning on `connect_changed` alone: a
    /// fresh buffer is already empty, so `set_text("")` is a no-op edit that
    /// never fires `changed`, and a structured pane would stay unpopulated.
    pub fn set_text(&self, text: &str) {
        self.imp().buffer.set_text(text);
        self.refresh_validity();
        self.redraw_filters();
        self.redraw_sync();
        self.redraw_ui();
        self.redraw_keys();
        self.schedule_write();
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
        // `[accounts]` is retired from the file (#470, ADR 0005 Q6b), so the
        // nav item stays and points at the thing that does work. The nav is
        // the panel's table of contents: someone who clicks `[accounts]`
        // wants accounts, and scrolling them to a section the validity line
        // calls ignored is the same lie one step further on.
        if section == Section::Accounts {
            imp.accounts_scroller.grab_focus();
            return;
        }
        if section == Section::Privacy {
            imp.privacy_scroller.grab_focus();
            return;
        }
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

        let mut status = format!(
            "{} · applied live · nothing to save",
            checked.validation.status_line()
        );
        let problems = imp.keymap_problems.borrow();
        if !problems.is_empty() {
            status.push_str(&format!(
                " · {} keymap {}: {}",
                problems.len(),
                if problems.len() == 1 {
                    "problem"
                } else {
                    "problems"
                },
                problems.join("; ")
            ));
        }
        imp.status.set_label(&status);
    }

    /// Tells the panel which `[keys]` bindings the resolver dropped, so they
    /// show up on the footer line -- the same place TOML validity does --
    /// rather than only in a debug log.
    ///
    /// `window.rs` calls this with `Resolver::apply_commands`'s and
    /// `Resolver::from_commands`'s return value on every keymap build and
    /// every live reload, whether or not this panel happens to be open.
    pub fn set_keymap_problems(&self, problems: &[String]) {
        *self.imp().keymap_problems.borrow_mut() = problems.to_vec();
        self.refresh_validity();
    }

    /// Shows one row per account, each with its enabled switch (#464).
    ///
    /// Hidden entirely when `accounts` is empty: a section with nothing in
    /// it is clutter the composer's signature picker already taught this
    /// codebase not to add.
    pub fn set_accounts(&self, accounts: Vec<Account>) {
        *self.imp().accounts.borrow_mut() = accounts;
        self.redraw_accounts();
    }

    /// What each account's mail weighs, and whether payloads are already
    /// being fetched.
    ///
    /// `attachments_included` is `[sync] attachment_fetch == "eager"`. The
    /// setting is global and these figures are per account, which is why the
    /// numbers land on the rows rather than beside the setting: there is no
    /// row to read a summed-across-accounts figure off, and this panel is a
    /// `TextView` over literal TOML on purpose -- a form control here would
    /// fight what it exists for (#411).
    ///
    /// Order-independent with [`set_accounts`](Self::set_accounts): whichever
    /// arrives second redraws the rows from both.
    pub fn set_mail_weights(
        &self,
        weights: &[(AccountId, postio_core::event::MailFootprint)],
        attachments_included: bool,
    ) {
        let imp = self.imp();
        *imp.weights.borrow_mut() = weights.to_vec();
        imp.attachments_included.set(attachments_included);
        self.redraw_accounts();
    }

    /// Every OAuth account's persisted token expiry (#878) — an id present
    /// with `None` means a token is on file but no provider-stated expiry
    /// is, which is a real answer distinct from having nothing to say at
    /// all: this panel may not link `postio-account` to fetch the keyring
    /// itself (the crate-boundary rule this widget's own module doc
    /// explains), so the composition root reads it and hands the result
    /// back the same way it hands back [`set_mail_weights`](Self::set_mail_weights)'s
    /// figures.
    ///
    /// Order-independent with [`set_accounts`](Self::set_accounts), for the
    /// same reason `set_mail_weights` is.
    pub fn set_token_expiries(&self, expiries: &[(AccountId, Option<std::time::SystemTime>)]) {
        *self.imp().token_expiries.borrow_mut() = expiries.to_vec();
        self.redraw_accounts();
    }

    /// The validity line this account's row carries under its badge, if it
    /// has one to carry — `None` for a password account, an OAuth account
    /// fed by an external broker, or one [`set_token_expiries`](Self::set_token_expiries)
    /// has not been told about yet.
    fn token_validity(&self, account: AccountId) -> Option<String> {
        let expiry = self
            .imp()
            .token_expiries
            .borrow()
            .iter()
            .find(|(id, _)| *id == account)
            .map(|(_, expiry)| *expiry)?;
        let at = expiry?;
        match at.duration_since(std::time::SystemTime::now()) {
            Ok(remaining) => {
                let days = remaining.as_secs() / (24 * 60 * 60);
                Some(if days == 0 {
                    "token valid less than a day".to_owned()
                } else {
                    format!("token valid {days}d")
                })
            }
            Err(_) => Some("token expired — re-authorization needed".to_owned()),
        }
    }

    /// Shows one row per sender with a standing remote-image exception,
    /// each with a way to revoke it (#871).
    ///
    /// `path` is where a revoke writes back to — `window.rs` hands in the
    /// same path [`Window::new_reader`](super::window::Window::new_reader)
    /// loads fresh on every call, so a revoke here reaches the next reader
    /// built after it. Already-open readers keep whatever they last loaded,
    /// same as any other file the watcher does not follow into a widget's
    /// own cache.
    pub fn set_remote_image_allowlist(
        &self,
        list: crate::reader::RemoteImageAllowList,
        path: PathBuf,
    ) {
        *self.imp().remote_image_allowlist.borrow_mut() = Some((list, path));
        self.redraw_privacy();
    }

    /// Rebuilds the privacy rows from whatever allow-list is held.
    fn redraw_privacy(&self) {
        let imp = self.imp();
        while let Some(row) = imp.privacy_list.row_at_index(0) {
            imp.privacy_list.remove(&row);
        }
        let senders: Vec<String> = imp
            .remote_image_allowlist
            .borrow()
            .as_ref()
            .map(|(list, _)| list.senders().map(str::to_owned).collect())
            .unwrap_or_default();
        for sender in &senders {
            imp.privacy_list.append(&self.privacy_row(sender));
        }
        imp.privacy_scroller.set_visible(!senders.is_empty());
        imp.privacy_empty.set_visible(senders.is_empty());
    }

    /// Revokes `sender`'s remote-image exception and writes the allow-list
    /// straight back — no debounce, unlike the config buffer: this is its
    /// own small file, not a keystroke-by-keystroke edit.
    fn revoke_remote_image_sender(&self, sender: &str) {
        let imp = self.imp();
        let mut guard = imp.remote_image_allowlist.borrow_mut();
        let Some((list, path)) = guard.as_mut() else {
            return;
        };
        list.revoke(sender);
        if let Err(error) = list.save_to(path) {
            tracing::error!(%error, "could not save the remote-image allow-list");
        }
        drop(guard);
        self.redraw_privacy();
    }

    /// One allow-listed sender, with a button to revoke it.
    fn privacy_row(&self, sender: &str) -> gtk::ListBoxRow {
        let sender = sender.to_string();
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-privacy-row");
        row.set_selectable(false);

        let label = gtk::Label::new(Some(&sender));
        label.add_css_class("postio-settings-privacy-sender");
        label.set_xalign(0.0);
        label.set_hexpand(true);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let revoke = gtk::Button::from_icon_name("user-trash-symbolic");
        revoke.add_css_class("postio-settings-privacy-revoke");
        revoke.add_css_class("flat");
        revoke.set_tooltip_text(Some("Always ask again"));
        revoke.update_property(&[gtk::accessible::Property::Label(&format!(
            "Stop always allowing remote images from {sender}"
        ))]);
        revoke.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            sender,
            move |_| panel.revoke_remote_image_sender(&sender)
        ));

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        box_.append(&label);
        box_.append(&revoke);
        row.set_child(Some(&box_));
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{sender}, always allowed to load remote images"
        ))]);
        row
    }

    /// Hands the panel the current account's unsubscribe-activation log
    /// (#971), newest first — `window.rs` reads it fresh from
    /// [`postio_storage::repository::UnsubscribeRepository`] every time the
    /// pane opens, the same reason [`SettingsPanel::set_remote_image_allowlist`]
    /// is handed its list rather than reading one itself: `postio-gtk` has
    /// no SQL of its own.
    pub fn set_unsubscribe_activations(&self, activations: Vec<UnsubscribeActivation>) {
        *self.imp().unsubscribe_activations.borrow_mut() = activations;
        self.redraw_unsubscribe_activations();
    }

    /// Hands the panel how many messages have asked for a read receipt
    /// (#970) — `window.rs` reads the count fresh from
    /// [`postio_storage::repository::MessageRepository::read_receipt_requested_count`]
    /// every time the pane opens, the same reason the two lists above are
    /// handed their state rather than reading it themselves.
    ///
    /// A count, not a switch: Postio never sends a receipt automatically
    /// (CLAUDE.md's privacy section calls that fixed policy), so a
    /// "configurable" default here would already have lost the argument a
    /// toggle exists to make.
    pub fn set_read_receipt_count(&self, count: u64) {
        let text = match count {
            0 => "No messages have requested a read receipt.".to_owned(),
            1 => "1 message has requested a read receipt; none have been sent \
                  automatically."
                .to_owned(),
            n => format!(
                "{n} messages have requested a read receipt; none have been \
                 sent automatically."
            ),
        };
        self.imp().read_receipt_count.set_label(&text);
    }

    /// The read-receipt count line's current text. For tests.
    #[doc(hidden)]
    pub fn read_receipt_count_label(&self) -> String {
        self.imp().read_receipt_count.label().to_string()
    }

    /// Rebuilds the unsubscribe-log rows from whatever was last handed in.
    fn redraw_unsubscribe_activations(&self) {
        let imp = self.imp();
        while let Some(row) = imp.unsubscribe_list.row_at_index(0) {
            imp.unsubscribe_list.remove(&row);
        }
        let activations = imp.unsubscribe_activations.borrow();
        for activation in activations.iter() {
            imp.unsubscribe_list
                .append(&self.unsubscribe_activation_row(activation));
        }
        imp.unsubscribe_scroller
            .set_visible(!activations.is_empty());
        imp.unsubscribe_empty.set_visible(activations.is_empty());
    }

    /// One past activation: the list it left, and when.
    fn unsubscribe_activation_row(&self, activation: &UnsubscribeActivation) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-unsubscribe-row");
        row.set_selectable(false);

        let list = gtk::Label::new(Some(&activation.list_identifier));
        list.add_css_class("postio-settings-unsubscribe-list-identifier");
        list.set_xalign(0.0);
        list.set_hexpand(true);
        list.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let when = activation.activated_at.format("%Y-%m-%d").to_string();
        let when_label = gtk::Label::new(Some(&when));
        when_label.add_css_class("postio-settings-unsubscribe-when");
        when_label.set_xalign(1.0);

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        box_.append(&list);
        box_.append(&when_label);
        row.set_child(Some(&box_));
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "Left {} on {when}",
            activation.list_identifier
        ))]);
        row
    }

    /// Rebuilds the account rows from whatever accounts and weights are held.
    fn redraw_accounts(&self) {
        let imp = self.imp();
        while let Some(row) = imp.accounts_list.row_at_index(0) {
            imp.accounts_list.remove(&row);
        }
        for account in imp.accounts.borrow().iter() {
            imp.accounts_list.append(&self.account_row(account));
        }
        // A refresh can land while the detail view is open on an account
        // this same redraw just found gone -- removed from another window,
        // most likely -- and showing an editable form over settings that no
        // longer exist would let an edit resurrect a deleted account.
        if let Some(id) = *imp.account_detail_id.borrow()
            && !imp.accounts.borrow().iter().any(|account| account.id == id)
        {
            self.close_account_detail();
            return;
        }
        // The detail view, not the list, owns this section's visibility
        // while it is open (#880) -- an ordinary refresh must not pop the
        // list back in front of it.
        if imp.account_detail_id.borrow().is_none() {
            imp.accounts_scroller
                .set_visible(!imp.accounts.borrow().is_empty());
        }
    }

    /// The sentence this account's row carries under its name, if it has one
    /// to carry.
    fn mail_weight(&self, account: AccountId) -> Option<String> {
        let imp = self.imp();
        let footprint = imp
            .weights
            .borrow()
            .iter()
            .find(|(id, _)| *id == account)
            .map(|(_, footprint)| *footprint)?;
        postio_ui::format::mail_weight(&footprint, imp.attachments_included.get())
    }

    /// The connections Postio has opened, newest first (#151).
    ///
    /// This list is the privacy claim made auditable: every outbound
    /// connection the transports report lands in the egress log, and this
    /// is where a person reads it back. Hidden while the log is empty —
    /// which on a machine that has never synced is exactly the claim.
    pub fn set_egress(&self, entries: Vec<postio_model::egress::EgressEvent>) {
        let imp = self.imp();
        while let Some(row) = imp.egress_list.row_at_index(0) {
            imp.egress_list.remove(&row);
        }
        for entry in &entries {
            let row = gtk::ListBoxRow::new();
            row.add_css_class("postio-settings-egress-row");
            row.set_selectable(false);
            row.set_activatable(false);
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            let when = gtk::Label::new(Some(
                &entry
                    .at
                    .with_timezone(&chrono::Local)
                    .format("%d %b %H:%M")
                    .to_string(),
            ));
            when.add_css_class("postio-settings-egress-when");
            let what = gtk::Label::new(Some(&format!(
                "{} · {}:{}",
                entry.subsystem.as_str(),
                entry.host,
                entry.port
            )));
            what.set_hexpand(true);
            what.set_xalign(0.0);
            what.set_ellipsize(gtk::pango::EllipsizeMode::End);
            let outcome = gtk::Label::new(Some(entry.outcome.as_str()));
            outcome.add_css_class("postio-settings-egress-outcome");
            line.append(&when);
            line.append(&what);
            line.append(&outcome);
            row.set_child(Some(&line));
            row.update_property(&[gtk::accessible::Property::Label(&format!(
                "{} connected to {} port {}, {}",
                entry.subsystem.as_str(),
                entry.host,
                entry.port,
                entry.outcome.as_str()
            ))]);
            imp.egress_list.append(&row);
        }
        imp.egress_scroller.set_visible(!entries.is_empty());
    }

    /// One account's row: name and address, what its mail weighs, and an
    /// enabled switch at the end.
    fn account_row(&self, account: &Account) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-account-row");
        row.set_selectable(false);
        // glib cannot know the type a key was stored under; this file can —
        // the same technique `Sidebar`'s rows use for their own ids (#292).
        #[allow(unsafe_code)]
        unsafe {
            row.set_data("postio-account-id", account.id.get());
        }

        let label = gtk::Label::new(Some(&format!(
            "{} ({})",
            account.display_name, account.address.address
        )));
        label.set_xalign(0.0);
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        // Name over number, in one column: the identity is what the row is
        // for and the size is a fact about it, so the second line is
        // secondary in the ordinary way rather than a second column
        // competing with the first.
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 0);
        lines.set_hexpand(true);
        lines.append(&label);
        let weight = self.mail_weight(account.id);
        if let Some(text) = &weight {
            let size = gtk::Label::new(Some(text));
            size.add_css_class("postio-settings-account-weight");
            size.set_xalign(0.0);
            size.set_ellipsize(gtk::pango::EllipsizeMode::End);
            lines.append(&size);
        }

        let badge_text = account_badge(account);
        let badge = gtk::Label::new(Some(&badge_text));
        badge.add_css_class("postio-settings-account-badge");
        badge.set_xalign(0.0);
        badge.set_ellipsize(gtk::pango::EllipsizeMode::End);
        lines.append(&badge);

        let validity = self.token_validity(account.id);
        if let Some(text) = &validity {
            let line = gtk::Label::new(Some(text));
            line.add_css_class("postio-settings-account-validity");
            line.set_xalign(0.0);
            line.set_ellipsize(gtk::pango::EllipsizeMode::End);
            lines.append(&line);
        }

        let enabled = gtk::Switch::new();
        enabled.set_active(account.enabled);
        enabled.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} enabled",
            account.display_name
        ))]);
        let account_id = account.id;
        enabled.connect_active_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |switch| {
                for callback in panel.imp().account_enabled_changed.borrow().iter() {
                    callback(account_id, switch.is_active());
                }
            }
        ));

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        box_.append(&lines);
        box_.append(&enabled);
        row.set_child(Some(&box_));
        // The row is announced as a unit, so every line has to be part of
        // the announcement or a screen reader never reaches it.
        let mut announcement = format!("{}, {}", account.display_name, account.address.address);
        if let Some(weight) = &weight {
            announcement.push_str(&format!(", {weight}"));
        }
        announcement.push_str(&format!(", {badge_text}"));
        if let Some(validity) = &validity {
            announcement.push_str(&format!(", {validity}"));
        }
        row.update_property(&[gtk::accessible::Property::Label(&announcement)]);
        row
    }

    /// Called when an account row's context menu picks
    /// [`AccountAction::UpdateCredential`] or [`AccountAction::Remove`].
    /// The account list itself, for the focus controller `Window` puts on it.
    ///
    /// A widget accessor rather than a `connect_` seam because what the
    /// window needs is the widget: `Context::Accounts` is scoped to exactly
    /// this list (ADR 0005 Q6c), and an `EventControllerFocus` has to go on
    /// the thing the context is named for.
    pub fn accounts_list(&self) -> gtk::ListBox {
        self.imp().accounts_list.clone()
    }

    /// The keybinding list itself, for the focus controller `Window` puts
    /// on it — the same reason [`accounts_list`](Self::accounts_list)
    /// exists, `Context::Keys` (#1016) in place of `Context::Accounts`.
    pub fn keys_list(&self) -> gtk::ListBox {
        self.imp().keys_list.clone()
    }

    /// The account whose row the keyboard is in, if it is in one.
    ///
    /// Focus rather than selection: the rows are `set_selectable(false)` and
    /// the list is `SelectionMode::None`, because an account row is a thing
    /// you act on rather than a thing you pick. `focus_child` is the row that
    /// contains the focus, which is what "the row the keyboard is on" means
    /// when the focus is actually on the switch inside it.
    ///
    /// `None` is a real answer and the callers must respect it: the context
    /// can be live with the focus somewhere else in the panel, and a command
    /// that guessed a row would remove an account on a keystroke aimed at
    /// nothing.
    pub fn focused_account(&self) -> Option<AccountId> {
        let row = self
            .imp()
            .accounts_list
            .focus_child()?
            .downcast::<gtk::ListBoxRow>()
            .ok()?;
        let id = row_account_id(&row);
        id.is_assigned().then_some(id)
    }

    /// Fires the account-action callbacks, as the row's context menu does.
    ///
    /// The keyboard path and the mouse path go through here together on
    /// purpose: two entry points that each call their own handlers are two
    /// things to keep in step, and this one ends in an account being removed.
    pub fn request_account_action(&self, id: AccountId, action: AccountAction) {
        for callback in self.imp().account_action.borrow().iter() {
            callback(id, action);
        }
    }

    /// Flips `id`'s enabled switch, as clicking it does.
    ///
    /// Moves the switch rather than calling the handler directly, so the
    /// control the person is looking at and the column the handler writes
    /// cannot disagree — the notify signal the switch emits is what calls
    /// the handler, exactly as it does for a click.
    ///
    /// Answers whether a row for `id` was found.
    pub fn toggle_account_enabled(&self, id: AccountId) -> bool {
        let Some(switch) = self.enabled_switch(id) else {
            return false;
        };
        switch.set_active(!switch.is_active());
        true
    }

    /// The enabled switch on `id`'s row.
    fn enabled_switch(&self, id: AccountId) -> Option<gtk::Switch> {
        let mut index = 0;
        while let Some(row) = self.imp().accounts_list.row_at_index(index) {
            index += 1;
            if row_account_id(&row) != id {
                continue;
            }
            let mut child = row.child()?.first_child();
            while let Some(widget) = child {
                if let Ok(switch) = widget.clone().downcast::<gtk::Switch>() {
                    return Some(switch);
                }
                child = widget.next_sibling();
            }
            return None;
        }
        None
    }

    pub fn connect_account_action(&self, handler: impl Fn(AccountId, AccountAction) + 'static) {
        self.imp()
            .account_action
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Called when a row's enabled switch is flipped by hand — never for the
    /// initial state [`SettingsPanel::set_accounts`] itself sets.
    pub fn connect_account_enabled_changed(&self, handler: impl Fn(AccountId, bool) + 'static) {
        self.imp()
            .account_enabled_changed
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Opens the detail view on `id`'s current settings, as activating its
    /// row does. Does nothing if `id` is not one of the accounts
    /// [`SettingsPanel::set_accounts`] last gave this panel.
    pub fn open_account_detail(&self, id: AccountId) {
        let imp = self.imp();
        let Some(account) = imp
            .accounts
            .borrow()
            .iter()
            .find(|account| account.id == id)
            .cloned()
        else {
            return;
        };
        self.ensure_account_detail_fields();
        imp.account_detail_loading.set(true);
        imp.account_detail_display_name
            .get()
            .expect("built above")
            .set_text(&account.display_name);
        imp.account_detail_imap_host
            .get()
            .expect("built above")
            .set_text(&account.incoming.host);
        imp.account_detail_imap_port
            .get()
            .expect("built above")
            .set_value(f64::from(account.incoming.port));
        imp.account_detail_smtp_host
            .get()
            .expect("built above")
            .set_text(&account.outgoing.host);
        // #979: the account's own signatures, and the one it already
        // prefers. Hidden entirely when it has none — the rule
        // `composer.rs::set_signatures` states and `set_accounts` cites: a
        // picker with nothing to choose between can only ever say what is
        // already true. Nothing in Postio creates a signature yet, so a
        // prompt to make one would point at a flow that does not exist.
        {
            let picker = imp.account_detail_signature.get().expect("built above");
            let names: Vec<&str> = account
                .signatures
                .iter()
                .map(|signature| signature.name.as_str())
                .collect();
            picker.set_model(Some(&gtk::StringList::new(&names)));
            *imp.account_detail_signature_ids.borrow_mut() =
                account.signatures.iter().map(|s| s.id).collect();
            let selected = account
                .default_signature_id
                .and_then(|id| account.signatures.iter().position(|s| s.id == id))
                .unwrap_or(0);
            picker.set_selected(selected as u32);
            imp.account_detail_signature_row
                .get()
                .expect("built above")
                .set_visible(!account.signatures.is_empty());
        }

        imp.account_detail_smtp_port
            .get()
            .expect("built above")
            .set_value(f64::from(account.outgoing.port));
        imp.account_detail_loading.set(false);
        *imp.account_detail_id.borrow_mut() = Some(id);
        imp.account_detail.set_visible(true);
        imp.accounts_scroller.set_visible(false);
    }

    /// Builds the detail view's five field widgets, the first time any
    /// account's detail is opened — never during `build()` or this
    /// widget's own construction.
    ///
    /// `SettingsPanel` is built as a hidden overlay child while `Window::new`
    /// is still wiring up its own overlay siblings and shortcut controllers
    /// (`window.rs`), and constructing a widget with its own internal event
    /// controllers there was found to corrupt keyboard routing for the rest
    /// of that window (#873, about a `gtk::DropDown`) — `gtk::Entry` and
    /// `gtk::SpinButton` carry the same kind of internal `GtkText`
    /// key/IM controllers a `DropDown`'s type-ahead does, so they get the
    /// same treatment: built only once a real interaction (opening an
    /// account's detail) proves the window has long since finished
    /// constructing.
    fn ensure_account_detail_fields(&self) {
        let imp = self.imp();
        if imp.account_detail_display_name.get().is_some() {
            return;
        }

        let display_name = gtk::Entry::new();
        display_name.add_css_class("postio-settings-account-detail-display-name");
        display_name.update_property(&[gtk::accessible::Property::Label("Display name")]);
        display_name.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| {
                panel.commit_account_edit(AccountEdit::DisplayName(entry.text().to_string()));
            }
        ));
        imp.account_detail
            .append(&detail_row("Display name", &display_name));
        let _ = imp.account_detail_display_name.set(display_name);

        let imap_host = gtk::Entry::new();
        imap_host.add_css_class("postio-settings-account-detail-imap-host");
        imap_host.update_property(&[gtk::accessible::Property::Label("IMAP host")]);
        imap_host.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| {
                panel.commit_account_edit(AccountEdit::ImapHost(entry.text().to_string()));
            }
        ));
        imp.account_detail
            .append(&detail_row("IMAP host", &imap_host));
        let _ = imp.account_detail_imap_host.set(imap_host);

        let imap_port = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
        imap_port.add_css_class("postio-settings-account-detail-imap-port");
        imap_port.update_property(&[gtk::accessible::Property::Label("IMAP port")]);
        imap_port.connect_value_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |spin| {
                panel.commit_account_edit(AccountEdit::ImapPort(spin.value() as u16));
            }
        ));
        imp.account_detail
            .append(&detail_row("IMAP port", &imap_port));
        let _ = imp.account_detail_imap_port.set(imap_port);

        let smtp_host = gtk::Entry::new();
        smtp_host.add_css_class("postio-settings-account-detail-smtp-host");
        smtp_host.update_property(&[gtk::accessible::Property::Label("SMTP host")]);
        smtp_host.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| {
                panel.commit_account_edit(AccountEdit::SmtpHost(entry.text().to_string()));
            }
        ));
        imp.account_detail
            .append(&detail_row("SMTP host", &smtp_host));
        let _ = imp.account_detail_smtp_host.set(smtp_host);

        let smtp_port = gtk::SpinButton::with_range(1.0, 65535.0, 1.0);
        smtp_port.add_css_class("postio-settings-account-detail-smtp-port");
        smtp_port.update_property(&[gtk::accessible::Property::Label("SMTP port")]);
        smtp_port.connect_value_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |spin| {
                panel.commit_account_edit(AccountEdit::SmtpPort(spin.value() as u16));
            }
        ));
        imp.account_detail
            .append(&detail_row("SMTP port", &smtp_port));
        let _ = imp.account_detail_smtp_port.set(smtp_port);

        // #979. A dropdown over the account's own signatures, not the
        // "signature path" field #880's mockup drew: `Account` carries
        // `signatures: Vec<Signature>` and `default_signature_id`, and there
        // has never been a filesystem path for that field to have edited.
        //
        // The row is built here and *hidden* per account in
        // `open_account_detail`, because whether it has anything to offer is
        // a fact about the account rather than about the panel.
        let signature = gtk::DropDown::from_strings(&[]);
        signature.add_css_class("postio-settings-account-detail-signature");
        signature.update_property(&[gtk::accessible::Property::Label("Default signature")]);
        signature.connect_selected_item_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |picker| {
                let chosen = panel
                    .imp()
                    .account_detail_signature_ids
                    .borrow()
                    .get(picker.selected() as usize)
                    .copied();
                panel.commit_account_edit(AccountEdit::DefaultSignature(chosen));
            }
        ));
        let signature_row = detail_row("Default signature", &signature);
        imp.account_detail.append(&signature_row);
        let _ = imp.account_detail_signature_row.set(signature_row);
        let _ = imp.account_detail_signature.set(signature);

        // "Test connection" (#980), under the servers it is about. The
        // status line lives beside it rather than in a toast: the result is
        // something a person reads, compares against the fields above, and
        // then edits, so it has to stay on screen next to them.
        let test = gtk::Button::with_label("Test connection");
        test.add_css_class("postio-settings-account-detail-test");
        // Its own width, not the row's. Every other control here is a field
        // the value fills; a full-bleed button reads as the primary action of
        // the whole screen, which this is not.
        test.set_halign(gtk::Align::Start);
        test.update_property(&[gtk::accessible::Property::Label(
            "Test connection to this account's servers",
        )]);
        test.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.ask_test_connection()
        ));
        imp.account_detail.append(&detail_row("Connection", &test));
        let _ = imp.account_detail_test_button.set(test);

        let status = gtk::Label::new(None);
        status.add_css_class("postio-settings-account-detail-test-status");
        status.set_xalign(0.0);
        status.set_wrap(true);
        // A live region: the result arrives a round trip after the press, so
        // a screen reader has to be told rather than having to be looking.
        status.update_property(&[gtk::accessible::Property::Label("")]);
        status.set_visible(false);
        imp.account_detail.append(&status);
        let _ = imp.account_detail_test_status.set(status);
    }

    /// Closes the detail view and shows the account list again, as the
    /// back button does.
    pub fn close_account_detail(&self) {
        let imp = self.imp();
        *imp.account_detail_id.borrow_mut() = None;
        imp.account_detail.set_visible(false);
        imp.accounts_scroller
            .set_visible(!imp.accounts.borrow().is_empty());
    }

    /// Called when a field in the account detail view is committed —
    /// `Enter` in an `Entry`, or any change to a `SpinButton`.
    /// Called when somebody asks whether an account's stored settings work,
    /// with the account the detail view is open on (#980).
    ///
    /// The panel never connects to anything. Same split as
    /// [`connect_account_edited`](Self::connect_account_edited): this layer
    /// may not link SQLite or open a socket, so it reports the gesture and
    /// `postio-app` runs `postio_session::reachability::test_connection` and
    /// hands the answer back through
    /// [`set_connection_status`](Self::set_connection_status).
    pub fn connect_test_connection(&self, handler: impl Fn(AccountId) + 'static) {
        self.imp()
            .test_connection
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Fires every `test_connection` handler for whichever account the detail
    /// view is open on, and puts the row into its running state so the press
    /// is acknowledged even if the answer takes a round trip.
    fn ask_test_connection(&self) {
        let Some(id) = *self.imp().account_detail_id.borrow() else {
            return;
        };
        self.set_connection_status(ConnectionStatus::Testing);
        for handler in self.imp().test_connection.borrow().iter() {
            handler(id);
        }
    }

    /// Show what the last connection test found.
    pub fn set_connection_status(&self, status: ConnectionStatus) {
        let Some(label) = self.imp().account_detail_test_status.get() else {
            return;
        };
        let message = status.message();
        label.set_visible(!message.is_empty());
        label.set_text(&message);
        // The same string to the screen reader: a status that is only a
        // colour is a status somebody cannot read.
        label.update_property(&[gtk::accessible::Property::Label(&message)]);
        // Failure is carried as a class rather than inferred from the text,
        // so the styling cannot disagree with the answer.
        if status.failed() {
            label.add_css_class("postio-settings-account-detail-test-failed");
        } else {
            label.remove_css_class("postio-settings-account-detail-test-failed");
        }
    }

    /// Press the test-connection button, as a pointer would. For tests.
    #[doc(hidden)]
    pub fn test_press_test_connection(&self) -> bool {
        match self.imp().account_detail_test_button.get() {
            Some(button) => {
                button.emit_clicked();
                true
            }
            None => false,
        }
    }

    /// What the connection status line currently says. For tests.
    #[doc(hidden)]
    pub fn test_connection_status_text(&self) -> String {
        self.imp()
            .account_detail_test_status
            .get()
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }

    pub fn connect_account_edited(&self, handler: impl Fn(AccountId, AccountEdit) + 'static) {
        self.imp()
            .account_edited
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Fires every `account_edited` handler with `edit`, against whichever
    /// account the detail view is currently open on. Silently does nothing
    /// while [`SettingsPanel::open_account_detail`] is populating the
    /// fields, and if the detail view is not open on anything at all —
    /// neither should happen from a real field commit, but a stray signal
    /// during a redraw is cheaper to ignore than to chase.
    fn commit_account_edit(&self, edit: AccountEdit) {
        let imp = self.imp();
        if imp.account_detail_loading.get() {
            return;
        }
        let Some(id) = *imp.account_detail_id.borrow() else {
            return;
        };
        for callback in imp.account_edited.borrow().iter() {
            callback(id, edit.clone());
        }
    }

    /// Opens an account row's context menu exactly as a right-click would,
    /// registering its actions on `self` — the same shape
    /// `Sidebar::test_open_saved_search_menu` uses, and for the same reason
    /// (GTK4 gives a test no way to simulate the click itself; see #424,
    /// #437). A test drives the result with
    /// `WidgetExt::activate_action("account.<verb>", None)`.
    #[doc(hidden)]
    pub fn test_open_account_menu(&self, x: f64, y: f64) {
        self.open_account_menu(x, y);
    }

    /// Closes the account row context menu, if one is open — see
    /// `Sidebar::test_close_saved_search_menu` for why a test must call this
    /// before tearing down the window.
    #[doc(hidden)]
    pub fn test_close_account_menu(&self) {
        if let Some(popover) = self.imp().account_menu.take() {
            popover.popdown();
        }
    }

    /// Open an account row's context menu at `(x, y)`, if there is a row
    /// there to open one for. See [`AccountAction`]'s own doc for why this
    /// is a fixed, hand-built menu rather than one the command registry
    /// generates.
    fn open_account_menu(&self, x: f64, y: f64) {
        let imp = self.imp();
        if let Some(previous) = imp.account_menu.take() {
            previous.popdown();
        }
        let Some(row) = imp.accounts_list.row_at_y(y as i32) else {
            return;
        };
        let id = row_account_id(&row);
        if !id.is_assigned() {
            return;
        }

        let menu = gtk::gio::Menu::new();
        menu.append(Some("Update credential"), Some("account.update-credential"));
        menu.append(Some("Remove"), Some("account.remove"));

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&imp.accounts_list);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        let actions = gtk::gio::SimpleActionGroup::new();
        for (name, action) in [
            ("update-credential", AccountAction::UpdateCredential),
            ("remove", AccountAction::Remove),
        ] {
            let simple = gtk::gio::SimpleAction::new(name, None);
            simple.connect_activate(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_, _| panel.request_account_action(id, action)
            ));
            actions.add_action(&simple);
        }
        // On `self`, not `imp.accounts_list`: matches `Sidebar`'s own reason
        // — the popover's items resolve the action by walking up from
        // wherever they are clicked, and inserting the group here is what
        // lets `test_open_account_menu` drive it through the public type.
        self.insert_action_group("account", Some(&actions));

        popover.connect_closed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[weak]
            popover,
            move |_| {
                popover.unparent();
                let current = panel.imp().account_menu.borrow().clone();
                if current.as_ref() == Some(&popover) {
                    panel.imp().account_menu.take();
                }
            }
        ));
        *imp.account_menu.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    /// Rebuilds the filter rows from the buffer's current text.
    ///
    /// Unlike accounts, `[filters]` lives entirely in `config.toml` — there
    /// is no second store to read, so this parses the buffer itself rather
    /// than waiting on an outside caller to hand over what to show. Invalid
    /// TOML mid-edit leaves whatever was last drawn rather than clearing it:
    /// a typo elsewhere in the file is not a reason to blank out a pane the
    /// user is not even looking at.
    fn redraw_filters(&self) {
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        while let Some(row) = imp.filters_list.row_at_index(0) {
            imp.filters_list.remove(&row);
        }
        let order = filter_display_order(&config);
        let pinned = config.ordered_filter_keys();
        for key in &order {
            imp.filters_list
                .append(&self.filter_row(key, &config.filters[key], &pinned));
        }
        imp.filters_scroller.set_visible(!order.is_empty());
        imp.filters_empty.set_visible(order.is_empty());
    }

    /// Applies `mutate` to the buffer's current `[filters]` state and writes
    /// the result back into the buffer — which is what actually reaches disk,
    /// through the same debounced write every other edit in this panel goes
    /// through. Invalid TOML mid-edit is left alone: there is no sensible
    /// `Config` to mutate yet.
    fn apply_filters_mutation(&self, mutate: impl FnOnce(&mut Config)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config);
        match patch_filters(&original, &config.filters) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [filters]"),
        }
    }

    /// Rebuilds `[sync]`'s five rows from the buffer's current text — the
    /// same fresh-widgets-each-time shape [`redraw_filters`](Self::redraw_filters)
    /// and [`redraw_ui`](Self::redraw_ui) use.
    fn redraw_sync(&self) {
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        while let Some(child) = imp.sync_box.first_child() {
            imp.sync_box.remove(&child);
        }
        imp.sync_box
            .append(&self.sync_check_for_mail_row(config.sync.check_for_mail));
        imp.sync_box
            .append(&self.sync_poll_interval_row(config.sync.poll_interval_secs));
        imp.sync_box
            .append(&self.sync_attachment_fetch_row(config.sync.attachment_fetch));
        imp.sync_box.append(&self.sync_notify_row(&config.sync));
    }

    /// Applies `mutate` to the buffer's current `[sync]` state and writes
    /// the result back into the buffer, the same way
    /// [`apply_filters_mutation`](Self::apply_filters_mutation) does for
    /// `[filters]`.
    fn apply_sync_mutation(&self, mutate: impl FnOnce(&mut SyncConfig)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config.sync);
        match patch_sync(&original, &config.sync) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [sync]"),
        }
    }

    /// How Postio learns about new mail: `Idle`/`Poll`/`Manual`, in that
    /// order — the same order [`CheckForMail`]'s own declaration gives.
    fn sync_check_for_mail_row(&self, current: CheckForMail) -> gtk::Box {
        let dropdown = gtk::DropDown::from_strings(&["IMAP IDLE", "Poll only", "Manual"]);
        dropdown.set_selected(match current {
            CheckForMail::Idle => 0,
            CheckForMail::Poll => 1,
            CheckForMail::Manual => 2,
        });
        dropdown.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |dropdown| {
                let check_for_mail = match dropdown.selected() {
                    1 => CheckForMail::Poll,
                    2 => CheckForMail::Manual,
                    _ => CheckForMail::Idle,
                };
                panel.apply_sync_mutation(move |sync| sync.check_for_mail = check_for_mail);
            }
        ));
        Self::sync_row(
            "Check for mail",
            "IMAP IDLE for push, polling only, or never on its own",
            &dropdown,
        )
    }

    /// Polling interval, in seconds — the fallback under IDLE and the only
    /// mechanism under Poll. 30 seconds to an hour covers every real
    /// server; a person who wants more has [`CheckForMail::Manual`].
    fn sync_poll_interval_row(&self, current: u64) -> gtk::Box {
        let spin = gtk::SpinButton::with_range(30.0, 3600.0, 30.0);
        spin.set_value(current as f64);
        spin.connect_value_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |spin| {
                let seconds = spin.value() as u64;
                panel.apply_sync_mutation(move |sync| sync.poll_interval_secs = seconds);
            }
        ));
        Self::sync_row(
            "Poll interval (seconds)",
            "How often a mailbox without IDLE, or every mailbox under Poll, is reconciled",
            &spin,
        )
    }

    /// When attachment payloads download: on open, eagerly, or never — ADR
    /// 0017's payload axis, in [`AttachmentFetch`]'s own declared order.
    fn sync_attachment_fetch_row(&self, current: AttachmentFetch) -> gtk::Box {
        let dropdown = gtk::DropDown::from_strings(&["On open", "Eager", "Never"]);
        dropdown.set_selected(match current {
            AttachmentFetch::OnOpen => 0,
            AttachmentFetch::Eager => 1,
            AttachmentFetch::Never => 2,
        });
        dropdown.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |dropdown| {
                let attachment_fetch = match dropdown.selected() {
                    1 => AttachmentFetch::Eager,
                    2 => AttachmentFetch::Never,
                    _ => AttachmentFetch::OnOpen,
                };
                panel.apply_sync_mutation(move |sync| sync.attachment_fetch = attachment_fetch);
            }
        ));
        Self::sync_row(
            "Download attachments",
            "On open is local-first: metadata syncs for every part, nothing downloads until opened",
            &dropdown,
        )
    }

    /// One row for both notification fields: the master switch, and which
    /// mailbox roles it covers as a comma-separated list — a plain text
    /// field rather than a multi-select, since the role names are the same
    /// stable identifiers the file already spells them with.
    fn sync_notify_row(&self, current: &SyncConfig) -> gtk::Box {
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);

        let switch = gtk::Switch::new();
        switch.set_active(current.notify);
        switch.set_valign(gtk::Align::Center);
        switch.update_property(&[gtk::accessible::Property::Label(
            "Desktop notifications for new mail",
        )]);
        switch.connect_active_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |switch| {
                let active = switch.is_active();
                panel.apply_sync_mutation(move |sync| sync.notify = active);
            }
        ));

        let roles = gtk::Entry::new();
        roles.set_text(&current.notify_roles.join(", "));
        roles.set_valign(gtk::Align::Center);
        roles.set_width_chars(18);
        roles.update_property(&[gtk::accessible::Property::Label(
            "Mailbox roles that notify",
        )]);
        roles.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| {
                let roles: Vec<String> = entry
                    .text()
                    .split(',')
                    .map(|role| role.trim().to_owned())
                    .filter(|role| !role.is_empty())
                    .collect();
                panel.apply_sync_mutation(move |sync| sync.notify_roles = roles);
            }
        ));

        controls.append(&switch);
        controls.append(&roles);
        Self::sync_row(
            "Notify for new mail",
            "Desktop notification, and which mailbox roles' arrivals notify",
            &controls,
        )
    }

    /// One labeled row: a title, a description, and a control at the end —
    /// the shape every structured settings row shares. `[ui]`'s own pane
    /// (#873) defines the same helper as `ui_row`; copied here rather than
    /// shared for the same reason `gtk_settings_filters.rs`'s own `collect`
    /// helper is copied instead of imported across test files: these are
    /// two sibling branches with no dependency between them yet, and a
    /// four-line layout helper is cheaper to duplicate once than to add a
    /// cross-issue dependency for.
    fn sync_row(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        let title_label = gtk::Label::new(Some(title));
        title_label.set_xalign(0.0);
        title_label.add_css_class("postio-settings-ui-title");
        lines.append(&title_label);
        let desc_label = gtk::Label::new(Some(description));
        desc_label.set_xalign(0.0);
        desc_label.add_css_class("postio-settings-ui-description");
        lines.append(&desc_label);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("postio-settings-ui-row");
        row.set_margin_top(7);
        row.set_margin_bottom(7);
        row.set_margin_start(18);
        row.set_margin_end(18);
        row.append(&lines);
        row.append(control);
        row
    }

    /// Rebuilds `[keys]`'s rows from the buffer's current text — the same
    /// fresh-widgets-each-time shape [`redraw_filters`](Self::redraw_filters)
    /// uses, one row per [`postio_core::registry::all`] entry rather than
    /// per file key: every command is rebindable whether or not `[keys]`
    /// currently overrides it.
    fn redraw_keys(&self) {
        self.ensure_capture_controller();
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        while let Some(row) = imp.keys_list.row_at_index(0) {
            imp.keys_list.remove(&row);
        }
        for spec in postio_core::registry::all() {
            imp.keys_list.append(&self.key_row(spec, &config.keys));
        }
    }

    /// Adds `keys_list`'s capture-phase `EventControllerKey`, the first
    /// time [`redraw_keys`](Self::redraw_keys) runs — never in `build()`.
    ///
    /// `SettingsPanel` is built as a hidden overlay child while `Window::new`
    /// is still wiring up its own overlay siblings and shortcut controllers
    /// (window.rs), and #873/#880 each found a widget with its own event
    /// controllers, built during that window, corrupting keyboard routing
    /// for the rest of it. This controller is not a composite widget's own
    /// internals the way those two cases were, so it is deferred out of
    /// `build()` on the same precautionary principle rather than because a
    /// `gtk_suite` regression was pinned on it specifically — a full-suite
    /// crash chased during this same issue turned out to be a pre-existing,
    /// machine-load-dependent flake, reproducible on `main` with none of
    /// this code present, not something this controller's timing caused or
    /// fixed. Deferring construction until a real interaction proves the
    /// window is done being built is cheap and has not been shown to be
    /// unnecessary, so it stays; see the issue thread for the bisection
    /// that cleared this controller instead of confirming it.
    fn ensure_capture_controller(&self) {
        let imp = self.imp();
        if imp.capture_controller_installed.get() {
            return;
        }
        imp.capture_controller_installed.set(true);

        // Capture phase: this must see a keypress before anything else in
        // the panel does, including the row's own rebind `Button`, or the
        // Space/Enter that presses that button would itself be swallowed
        // by the button's own activation instead of reaching capture.
        // Stops propagation only while actually capturing, so ordinary
        // navigation (Tab between rows, arrow keys in the list) is
        // untouched otherwise.
        let capture = gtk::EventControllerKey::new();
        capture.set_propagation_phase(gtk::PropagationPhase::Capture);
        capture.connect_key_pressed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[upgrade_or]
            glib::Propagation::Proceed,
            move |_, keyval, _, state| {
                if panel.imp().capturing.borrow().is_none() {
                    return glib::Propagation::Proceed;
                }
                // Held for one turn of the main loop rather than resolved
                // here: this fires mid-propagation, before the event has
                // finished being delivered to whatever has focus (usually
                // the capturing row's own rebind button) -- and resolving
                // synchronously means `redraw_keys` tears down that exact
                // widget while GTK is still routing the event to it.
                // `MessageList::hold` (list.rs) holds for the same reason:
                // let delivery finish, then act.
                glib::idle_add_local_once(glib::clone!(
                    #[weak]
                    panel,
                    move || panel.resolve_capture(keyval, state)
                ));
                glib::Propagation::Stop
            }
        ));
        imp.keys_list.add_controller(capture);
    }

    /// Applies `mutate` to the buffer's current `[keys]` overrides and
    /// writes the result back into the buffer, the same shape
    /// [`apply_filters_mutation`](Self::apply_filters_mutation) and
    /// [`apply_sync_mutation`](Self::apply_sync_mutation) already use.
    fn apply_keys_mutation(
        &self,
        mutate: impl FnOnce(&mut std::collections::BTreeMap<String, String>),
    ) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        let mut overrides = config.keys.overrides().clone();
        mutate(&mut overrides);
        *config.keys.overrides_mut() = overrides.clone();
        match patch_keys(&original, &overrides) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [keys]"),
        }
    }

    /// One command: its title, current effective binding, a rebind button,
    /// and — only right after a rejected capture on this exact command —
    /// why it was rejected.
    fn key_row(
        &self,
        spec: &postio_core::CommandSpec,
        bindings: &postio_config::KeyBindings,
    ) -> gtk::ListBoxRow {
        let command = spec.id;
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-keys-row");
        row.set_selectable(false);

        let title = gtk::Label::new(Some(spec.title));
        title.add_css_class("postio-settings-keys-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);

        let capturing = self
            .imp()
            .capturing
            .borrow()
            .is_some_and(|id| id == command);
        let current = bindings
            .binding(command.as_str())
            .unwrap_or(spec.default_binding)
            .to_owned();
        let binding = gtk::Label::new(Some(if capturing {
            "press a key…"
        } else {
            current.as_str()
        }));
        binding.add_css_class("postio-settings-keys-binding");
        binding.set_xalign(0.0);

        let rebind = gtk::Button::with_label(if capturing { "Cancel" } else { "Rebind" });
        rebind.add_css_class("postio-settings-keys-rebind");
        rebind.add_css_class("flat");
        rebind.update_property(&[gtk::accessible::Property::Label(&format!(
            "Rebind {}, currently {current}",
            spec.title
        ))]);
        rebind.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.toggle_capture(command)
        ));

        let conflict_text = self
            .imp()
            .capture_conflict
            .borrow()
            .as_ref()
            .filter(|(id, _)| *id == command)
            .map(|(_, message)| message.clone());
        let conflict = gtk::Label::new(conflict_text.as_deref());
        conflict.add_css_class("postio-settings-keys-conflict");
        conflict.set_xalign(0.0);
        conflict.set_wrap(true);
        conflict.set_visible(conflict_text.is_some());

        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        lines.append(&title);
        lines.append(&binding);
        lines.append(&conflict);

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        box_.append(&lines);
        box_.append(&rebind);
        row.set_child(Some(&box_));
        row
    }

    /// Enters or leaves capture mode for `command`, as its rebind button
    /// asks — the second click undoes the first, since nothing has
    /// happened yet to undo.
    fn toggle_capture(&self, command: CommandId) {
        let imp = self.imp();
        let already = *imp.capturing.borrow() == Some(command);
        *imp.capturing.borrow_mut() = if already { None } else { Some(command) };
        *imp.capture_conflict.borrow_mut() = None;
        self.redraw_keys();
        if !already {
            imp.keys_list.grab_focus();
        }
    }

    /// Resolves whatever [`toggle_capture`](Self::toggle_capture) is
    /// waiting on with a real keypress — the capture controller's own
    /// handler, and [`test_capture_key`](Self::test_capture_key)'s.
    ///
    /// `Escape` always cancels rather than becoming the new binding: the
    /// alternative would make "I want out of this" indistinguishable from
    /// "bind Escape here", and every other capture flow in the wild treats
    /// it as cancel.
    fn resolve_capture(&self, keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) {
        let Some(command) = *self.imp().capturing.borrow() else {
            return;
        };
        if keyval == gtk::gdk::Key::Escape {
            *self.imp().capturing.borrow_mut() = None;
            self.redraw_keys();
            return;
        }
        let Some(chord) = Chord::from_key_event(keyval, state) else {
            // A key this build has no name for -- stay in capture mode and
            // wait for a real one, the same as pressing a bare modifier.
            return;
        };
        let proposed = chord.to_string();

        let original = self.text();
        let Ok(config) = Config::from_toml_str(&original) else {
            *self.imp().capturing.borrow_mut() = None;
            self.redraw_keys();
            return;
        };
        if let Some(other) = postio_core::registry::binding_conflict(
            command,
            &proposed,
            &config.keys,
            postio_config::paths::Platform::host(),
        ) {
            *self.imp().capturing.borrow_mut() = None;
            *self.imp().capture_conflict.borrow_mut() =
                Some((command, format!("Already used by {}", other.title)));
            self.redraw_keys();
            return;
        }

        *self.imp().capturing.borrow_mut() = None;
        self.apply_keys_mutation(move |overrides| {
            overrides.insert(command.as_str().to_owned(), proposed);
        });
        self.redraw_keys();
    }

    /// Feeds a keypress to whichever command's row is capturing, as a real
    /// key controller would. `#[doc(hidden)]` because it exists only for
    /// tests: [`postio_gtk::window::Window::handle_key`](crate::window::Window::handle_key)
    /// resolves a synthetic keypress against the app's own resolver
    /// directly rather than dispatching a real `GdkEvent`, so it never
    /// reaches this panel's own capture controller — this is the seam that
    /// exercises the same logic that controller calls, the same trade
    /// [`SettingsPanel::test_open_account_menu`](Self::test_open_account_menu)
    /// already makes for a right click a test cannot reliably land on a
    /// pixel.
    #[doc(hidden)]
    pub fn test_capture_key(&self, keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) {
        self.resolve_capture(keyval, state);
    }

    /// Rebuilds `[ui]`'s six rows from the buffer's current text — the same
    /// shape [`redraw_filters`](Self::redraw_filters) uses, and for the same
    /// reason: fresh widgets each time means the initial value never fires
    /// its own change handler (set before connect, the way `account_row`'s
    /// switch already does), so there is no separate "is this a programmatic
    /// update" guard to keep in step.
    fn redraw_ui(&self) {
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        while let Some(child) = imp.ui_box.first_child() {
            imp.ui_box.remove(&child);
        }
        imp.ui_box.append(&self.ui_density_row(config.ui.density));
        imp.ui_box.append(&self.ui_theme_row(config.ui.theme));
        imp.ui_box.append(&self.ui_switch_row(
            "Show hover actions",
            "Reveal per-row actions when the pointer rests over a row",
            config.ui.show_hover_actions,
            |ui, value| ui.show_hover_actions = value,
        ));
        imp.ui_box.append(&self.ui_switch_row(
            "Show key hints",
            "The focused row's own keyboard hints",
            config.ui.show_key_hints,
            |ui, value| ui.show_key_hints = value,
        ));
        imp.ui_box.append(&self.ui_switch_row(
            "Show sender avatars",
            "An initials chip per row",
            config.ui.sender_avatars,
            |ui, value| ui.sender_avatars = value,
        ));
    }

    /// Applies `mutate` to the buffer's current `[ui]` state and writes the
    /// result back into the buffer, the same way
    /// [`apply_filters_mutation`](Self::apply_filters_mutation) does for
    /// `[filters]`.
    fn apply_ui_mutation(&self, mutate: impl FnOnce(&mut postio_config::UiConfig)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config.ui);
        match patch_ui(&original, &config.ui) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [ui]"),
        }
    }

    /// One labeled row: a title, a description, and a control at the end —
    /// the shape every `[ui]` row shares, whether the control is a dropdown
    /// or a switch.
    fn ui_row(title: &str, description: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        let title_label = gtk::Label::new(Some(title));
        title_label.set_xalign(0.0);
        title_label.add_css_class("postio-settings-ui-title");
        lines.append(&title_label);
        let desc_label = gtk::Label::new(Some(description));
        desc_label.set_xalign(0.0);
        desc_label.add_css_class("postio-settings-ui-description");
        lines.append(&desc_label);

        let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        row.add_css_class("postio-settings-ui-row");
        row.set_margin_top(7);
        row.set_margin_bottom(7);
        row.set_margin_start(18);
        row.set_margin_end(18);
        row.append(&lines);
        row.append(control);
        row
    }

    /// Message-list row height: `Airy`/`Comfortable`/`Compact`, in that
    /// order — the same order [`Density`]'s own default-first declaration
    /// gives, so the dropdown's index and the enum's discriminant agree
    /// without a lookup table.
    fn ui_density_row(&self, current: Density) -> gtk::Box {
        let dropdown = gtk::DropDown::from_strings(&["Airy", "Comfortable", "Compact"]);
        dropdown.set_selected(match current {
            Density::Airy => 0,
            Density::Comfortable => 1,
            Density::Compact => 2,
        });
        dropdown.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |dropdown| {
                let density = match dropdown.selected() {
                    1 => Density::Comfortable,
                    2 => Density::Compact,
                    _ => Density::Airy,
                };
                panel.apply_ui_mutation(move |ui| ui.density = density);
            }
        ));
        Self::ui_row(
            "Message-list row height",
            "Airy, comfortable, or compact",
            &dropdown,
        )
    }

    /// Light/dark preference: `System`/`Light`/`Dark`, same index convention
    /// as [`ui_density_row`](Self::ui_density_row).
    fn ui_theme_row(&self, current: Theme) -> gtk::Box {
        let dropdown = gtk::DropDown::from_strings(&["System", "Light", "Dark"]);
        dropdown.set_selected(match current {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        });
        dropdown.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |dropdown| {
                let theme = match dropdown.selected() {
                    1 => Theme::Light,
                    2 => Theme::Dark,
                    _ => Theme::System,
                };
                panel.apply_ui_mutation(move |ui| ui.theme = theme);
            }
        ));
        Self::ui_row(
            "Theme",
            "System follows the desktop's light/dark setting",
            &dropdown,
        )
    }

    /// One boolean `[ui]` row: a switch whose initial state is set before
    /// its change handler is connected, so redrawing from a fresh read
    /// never writes back the value it just read.
    fn ui_switch_row(
        &self,
        title: &str,
        description: &str,
        value: bool,
        apply: impl Fn(&mut postio_config::UiConfig, bool) + 'static,
    ) -> gtk::Box {
        let switch = gtk::Switch::new();
        switch.set_active(value);
        switch.set_valign(gtk::Align::Center);
        switch.update_property(&[gtk::accessible::Property::Label(title)]);
        switch.connect_active_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |switch| {
                let active = switch.is_active();
                panel.apply_ui_mutation(|ui| apply(ui, active));
            }
        ));
        Self::ui_row(title, description, &switch)
    }

    /// One saved search's row: its name (editable), its query (for context,
    /// not editable here — renaming a filter's query is not a feature this
    /// pane offers), whether it shows in the sidebar, reorder, and delete.
    ///
    /// `pinned_keys` is [`Config::ordered_filter_keys`] — the sidebar's own
    /// order — so the up/down buttons can tell a row apart from the very
    /// first or last *pinned* filter, which is not the same thing as this
    /// row's position in the combined pinned-then-unpinned list this pane
    /// displays.
    fn filter_row(
        &self,
        key: &str,
        filter: &FilterConfig,
        pinned_keys: &[String],
    ) -> gtk::ListBoxRow {
        // Owned, not borrowed: every row action below moves its own clone of
        // this into a `'static` closure, which a `&str` tied to the caller's
        // stack frame cannot satisfy.
        let key = key.to_string();
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-filter-row");
        row.set_selectable(false);

        let title = filter.name.clone().unwrap_or_else(|| filter.query.clone());
        let name_entry = gtk::Entry::new();
        name_entry.set_text(&title);
        name_entry.add_css_class("postio-settings-filter-name");
        name_entry.set_hexpand(true);
        name_entry.update_property(&[gtk::accessible::Property::Label(&format!(
            "Name for the saved search {}",
            filter.query
        ))]);
        name_entry.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            key,
            move |entry| {
                let key = key.clone();
                let text = entry.text().to_string();
                panel.apply_filters_mutation(move |config| {
                    config.rename_filter(&key, &text);
                });
            }
        ));

        let query_label = gtk::Label::new(Some(&filter.query));
        query_label.add_css_class("postio-settings-filter-query");
        query_label.set_xalign(0.0);
        query_label.set_ellipsize(gtk::pango::EllipsizeMode::End);

        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        lines.append(&name_entry);
        lines.append(&query_label);

        let pinned = gtk::Switch::new();
        pinned.set_active(filter.pinned);
        pinned.update_property(&[gtk::accessible::Property::Label(&format!(
            "Show {title} in the sidebar"
        ))]);
        pinned.connect_active_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            key,
            move |switch| {
                let key = key.clone();
                let active = switch.is_active();
                panel.apply_filters_mutation(move |config| {
                    config.set_filter_pinned(&key, active);
                });
            }
        ));

        let position = pinned_keys.iter().position(|candidate| *candidate == key);
        let up = gtk::Button::from_icon_name("go-up-symbolic");
        up.add_css_class("postio-settings-filter-up");
        up.add_css_class("flat");
        up.set_tooltip_text(Some("Move up"));
        up.set_sensitive(position.is_some_and(|index| index > 0));
        up.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            key,
            move |_| {
                let key = key.clone();
                panel.apply_filters_mutation(move |config| {
                    config.move_filter(&key, Reorder::Up);
                });
            }
        ));

        let down = gtk::Button::from_icon_name("go-down-symbolic");
        down.add_css_class("postio-settings-filter-down");
        down.add_css_class("flat");
        down.set_tooltip_text(Some("Move down"));
        down.set_sensitive(position.is_some_and(|index| index + 1 < pinned_keys.len()));
        down.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            key,
            move |_| {
                let key = key.clone();
                panel.apply_filters_mutation(move |config| {
                    config.move_filter(&key, Reorder::Down);
                });
            }
        ));

        let delete = gtk::Button::from_icon_name("user-trash-symbolic");
        delete.add_css_class("postio-settings-filter-delete");
        delete.add_css_class("flat");
        delete.set_tooltip_text(Some("Delete"));
        delete.update_property(&[gtk::accessible::Property::Label(&format!(
            "Delete the saved search {title}"
        ))]);
        delete.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            key,
            move |_| {
                let key = key.clone();
                panel.apply_filters_mutation(move |config| {
                    config.delete_filter(&key);
                });
            }
        ));

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        box_.set_margin_top(6);
        box_.set_margin_bottom(6);
        box_.set_margin_start(12);
        box_.set_margin_end(12);
        box_.append(&lines);
        box_.append(&pinned);
        box_.append(&up);
        box_.append(&down);
        box_.append(&delete);
        row.set_child(Some(&box_));
        row.update_property(&[gtk::accessible::Property::Label(&format!(
            "{title}, {}",
            filter.query
        ))]);
        row
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

        // ── accounts: one row each, an enable switch, a context menu ─────
        imp.accounts_list
            .add_css_class("postio-settings-accounts-list");
        imp.accounts_list
            .set_selection_mode(gtk::SelectionMode::None);
        imp.accounts_list
            .update_property(&[gtk::accessible::Property::Label("Accounts")]);

        let accounts_menu = gtk::GestureClick::new();
        accounts_menu.set_button(gtk::gdk::BUTTON_SECONDARY);
        accounts_menu.set_propagation_phase(gtk::PropagationPhase::Capture);
        accounts_menu.connect_pressed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, _, x, y| {
                panel.open_account_menu(x, y);
            }
        ));
        imp.accounts_list.add_controller(accounts_menu);

        // Activating a row (click, or Enter/Space when focused) opens the
        // detail view (#880) -- the switch and the context menu each
        // consume their own click before it would reach the row, so this
        // does not fire when either of those is what was actually pressed.
        imp.accounts_list.set_activate_on_single_click(true);
        imp.accounts_list.connect_row_activated(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                let id = row_account_id(row);
                if id.is_assigned() {
                    panel.open_account_detail(id);
                }
            }
        ));

        imp.accounts_scroller.set_child(Some(&imp.accounts_list));
        imp.accounts_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.accounts_scroller
            .set_max_content_height(ACCOUNTS_MAX_HEIGHT);
        imp.accounts_scroller.set_propagate_natural_height(true);
        imp.accounts_scroller
            .add_css_class("postio-settings-accounts");
        imp.accounts_scroller.set_visible(false);

        // ── account detail: display name, IMAP/SMTP host+port (#880) ─────
        imp.account_detail
            .add_css_class("postio-settings-account-detail");
        imp.account_detail.set_visible(false);

        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.add_css_class("postio-settings-account-detail-back");
        back.add_css_class("flat");
        back.set_halign(gtk::Align::Start);
        back.set_tooltip_text(Some("Back to accounts"));
        back.update_property(&[gtk::accessible::Property::Label("Back to accounts")]);
        back.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.close_account_detail()
        ));
        imp.account_detail.append(&back);
        // The five field widgets (Entry/SpinButton) are deliberately NOT
        // built here -- see `ensure_account_detail_fields`'s own doc for
        // why constructing them this early would repeat #873.

        // ── filters: one row each, name/query, pinned, reorder, delete ───
        imp.filters_list
            .add_css_class("postio-settings-filters-list");
        imp.filters_list
            .set_selection_mode(gtk::SelectionMode::None);
        imp.filters_list
            .update_property(&[gtk::accessible::Property::Label("Saved searches")]);

        imp.filters_scroller.set_child(Some(&imp.filters_list));
        imp.filters_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.filters_scroller
            .set_max_content_height(ACCOUNTS_MAX_HEIGHT);
        imp.filters_scroller.set_propagate_natural_height(true);
        imp.filters_scroller
            .add_css_class("postio-settings-filters");
        imp.filters_scroller.set_visible(false);

        imp.filters_empty
            .add_css_class("postio-settings-filters-empty");
        imp.filters_empty.set_xalign(0.0);
        imp.filters_empty.set_wrap(true);
        imp.filters_empty.set_visible(false);

        // ── sync: five rows, always present (#874) ────────────────────────
        imp.sync_box.add_css_class("postio-settings-sync");
        imp.sync_box
            .update_property(&[gtk::accessible::Property::Label("Sync & storage")]);

        // ── ui: six rows, always present (#873) ───────────────────────────
        imp.ui_box.add_css_class("postio-settings-ui");
        imp.ui_box
            .update_property(&[gtk::accessible::Property::Label("Appearance")]);

        // ── privacy: one row per allow-listed sender (#871) ───────────────
        imp.privacy_list
            .add_css_class("postio-settings-privacy-list");
        imp.privacy_list
            .set_selection_mode(gtk::SelectionMode::None);
        imp.privacy_list
            .update_property(&[gtk::accessible::Property::Label(
                "Senders always allowed to load remote images",
            )]);

        imp.privacy_scroller.set_child(Some(&imp.privacy_list));
        imp.privacy_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.privacy_scroller
            .set_max_content_height(ACCOUNTS_MAX_HEIGHT);
        imp.privacy_scroller.set_propagate_natural_height(true);
        imp.privacy_scroller
            .add_css_class("postio-settings-privacy");
        imp.privacy_scroller.set_visible(false);

        imp.privacy_empty
            .add_css_class("postio-settings-privacy-empty");
        imp.privacy_empty.set_xalign(0.0);
        imp.privacy_empty.set_wrap(true);
        imp.privacy_empty.set_visible(false);

        // ── privacy: one row per past unsubscribe activation (#971) ──────
        // A second list under the same nav section as `privacy_list`, so it
        // gets its own heading to tell the two apart — the only place in
        // this panel a section holds two lists.
        let unsubscribe_title = gtk::Label::new(Some("Mailing lists left"));
        unsubscribe_title.add_css_class("postio-settings-unsubscribe-title");
        unsubscribe_title.set_xalign(0.0);

        imp.unsubscribe_list
            .add_css_class("postio-settings-unsubscribe-list");
        imp.unsubscribe_list
            .set_selection_mode(gtk::SelectionMode::None);
        imp.unsubscribe_list
            .update_property(&[gtk::accessible::Property::Label(
                "Mailing lists left through one-click unsubscribe",
            )]);

        imp.unsubscribe_scroller
            .set_child(Some(&imp.unsubscribe_list));
        imp.unsubscribe_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.unsubscribe_scroller
            .set_max_content_height(ACCOUNTS_MAX_HEIGHT);
        imp.unsubscribe_scroller.set_propagate_natural_height(true);
        imp.unsubscribe_scroller
            .add_css_class("postio-settings-unsubscribe");
        imp.unsubscribe_scroller.set_visible(false);

        imp.unsubscribe_empty
            .add_css_class("postio-settings-unsubscribe-empty");
        imp.unsubscribe_empty.set_xalign(0.0);
        imp.unsubscribe_empty.set_wrap(true);
        imp.unsubscribe_empty.set_visible(false);

        // ── privacy: the read-receipt count, a fact rather than a toggle
        // (#970) ───────────────────────────────────────────────────────
        imp.read_receipt_count
            .add_css_class("postio-settings-read-receipt-count");
        imp.read_receipt_count.set_xalign(0.0);
        imp.read_receipt_count.set_wrap(true);
        self.set_read_receipt_count(0);

        // ── keys: one row per command, a rebind capture button (#881) ────
        imp.keys_list.add_css_class("postio-settings-keys-list");
        imp.keys_list.set_selection_mode(gtk::SelectionMode::None);
        imp.keys_list
            .update_property(&[gtk::accessible::Property::Label("Keybindings")]);

        // The capture controller is deliberately NOT built here -- see
        // `ensure_capture_controller`'s own doc for why.

        imp.keys_scroller.set_child(Some(&imp.keys_list));
        imp.keys_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.keys_scroller
            .set_max_content_height(ACCOUNTS_MAX_HEIGHT);
        imp.keys_scroller.set_propagate_natural_height(true);
        imp.keys_scroller.add_css_class("postio-settings-keys");
        // Unlike `accounts_scroller`/`filters_scroller`/`privacy_scroller`,
        // this one is never hidden -- the registry is never empty -- so a
        // screen reader always reaches it and needs something to say, the
        // same reason `nav_scroller`/`view_scroller` already announce
        // themselves.
        imp.keys_scroller
            .update_property(&[gtk::accessible::Property::Label("Keybindings")]);

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

        // ── egress: the connections Postio opened, auditable (#151) ──────
        imp.egress_list.add_css_class("postio-settings-egress-list");
        imp.egress_list.set_selection_mode(gtk::SelectionMode::None);
        imp.egress_list
            .update_property(&[gtk::accessible::Property::Label("Recent connections")]);
        imp.egress_scroller.set_child(Some(&imp.egress_list));
        imp.egress_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.egress_scroller
            .set_max_content_height(ACCOUNTS_MAX_HEIGHT);
        imp.egress_scroller.set_propagate_natural_height(true);
        imp.egress_scroller.add_css_class("postio-settings-egress");
        imp.egress_scroller.set_visible(false);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&header);
        column.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        column.append(&imp.accounts_scroller);
        column.append(&imp.account_detail);
        column.append(&imp.egress_scroller);
        column.append(&imp.filters_scroller);
        column.append(&imp.filters_empty);
        column.append(&imp.sync_box);
        column.append(&imp.ui_box);
        column.append(&imp.privacy_scroller);
        column.append(&imp.privacy_empty);
        column.append(&unsubscribe_title);
        column.append(&imp.unsubscribe_scroller);
        column.append(&imp.unsubscribe_empty);
        column.append(&imp.read_receipt_count);
        column.append(&imp.keys_scroller);
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
                panel.redraw_filters();
                panel.redraw_sync();
                panel.redraw_ui();
                panel.redraw_keys();
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
        self.redraw_filters();
        // Deliberately not redraw_sync()/redraw_ui() here: `Window::new`
        // constructs a `SettingsPanel` as a hidden overlay child while it is
        // still wiring up its own overlay siblings and shortcut controllers
        // (window.rs), and building a `gtk::DropDown` mid-construction there
        // was found to corrupt keyboard routing for the rest of that same
        // window -- gtk_finder, gtk_finder_focus, gtk_move_picker and
        // gtk_toggle_sidebar all failed until this was removed (#873). Both
        // panes populate lazily instead, from `load()`/`set_text()` once the
        // window construction they're part of has finished; `redraw_filters`
        // stays here because it builds no `DropDown` and never reproduced
        // the corruption.
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

    #[test]
    fn find_section_is_none_for_privacy_no_matter_what_the_file_says() {
        // Privacy never has a `[privacy]` header to find, in any file --
        // it is not a table at all, unlike `Filters` above which simply
        // has not been written yet.
        assert_eq!(find_section(SAMPLE, Section::Privacy), None);
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

    // -- filter_display_order (#869) -----------------------------------------

    #[test]
    fn filter_display_order_puts_pinned_filters_first_in_sidebar_order_then_unpinned_alphabetically()
     {
        let config = Config::from_toml_str(
            "\
[filters.b]
query = \"subject:b\"
pinned = false

[filters.zebra]
query = \"is:unread\"
pinned = true
order = 1

[filters.apple]
query = \"has:attach\"
pinned = true
order = 0

[filters.a]
query = \"subject:a\"
pinned = false
",
        )
        .expect("parses");

        assert_eq!(
            filter_display_order(&config),
            vec![
                "apple".to_string(),
                "zebra".to_string(),
                "a".to_string(),
                "b".to_string(),
            ],
            "pinned filters in their explicit order, then unpinned ones by key"
        );
    }

    // -- display_path -------------------------------------------------------

    #[test]
    fn a_path_under_home_is_shown_with_a_tilde() {
        // SAFETY: single-threaded test; nothing else reads `HOME` here.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("HOME", "/home/example")
        };
        assert_eq!(
            display_path(Path::new("/home/example/.config/postio/config.toml")),
            "~/.config/postio/config.toml"
        );
    }

    #[test]
    fn a_path_outside_home_is_shown_verbatim() {
        // SAFETY: single-threaded test; nothing else reads `HOME` here.
        #[allow(unsafe_code)]
        unsafe {
            std::env::set_var("HOME", "/home/example")
        };
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
