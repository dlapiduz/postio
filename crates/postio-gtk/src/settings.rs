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
//! [`Section::Filters`] and [`Section::Appearance`] are *forms* over the same file —
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
use gtk::{glib, pango};
use postio_config::compose::SignaturePlacement;
use postio_config::filters::{FilterConfig, Reorder};
use postio_config::sync::{AttachmentFetch, CheckForMail};
use postio_config::{
    Config, Density, SyncConfig, Theme, patch_compose, patch_filters, patch_keys, patch_sync,
    patch_ui,
};
use postio_core::CommandId;
use postio_model::ids::SignatureId;
use postio_model::{Account, AccountId, UnsubscribeActivation};

use crate::keymap::{Chord, ChordFromGdk};
use crate::widgets::{CheckRow, SegmentedControl, kicker, stat_line};

/// How long to let typing settle before writing the buffer back to disk.
///
/// Long enough that a fast typist is not racing the disk on every keystroke;
/// short enough that "applied live" still reads as true. The file watcher's
/// own debounce (`postio_config::watch::DEFAULT_DEBOUNCE`, 120ms) runs after
/// this one settles, so the whole round trip — keystroke to reload — is well
/// under half a second.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(250);

/// What the `Every 5 min` segment means in seconds.
///
/// Only ever written when the mode *changes* to polling — see
/// [`SettingsPanel::ensure_sync_controls`] for why a person's own interval
/// survives pressing the segment it is already on.
const POLL_EVERY_FIVE_MINUTES: u64 = 300;

/// How wide a row is measured at when the Appearance pane asks how tall the
/// chosen density makes one.
///
/// The list is windowed and its real width varies with the window, but row
/// height does not depend on width until the subject has to wrap, and it
/// never wraps — it ellipsizes. So any realistic width gives the same
/// answer, and a fixed one keeps the figure from flickering as the window
/// is dragged.
const DENSITY_PROBE_WIDTH: i32 = 360;

/// How wide the sidebar is — fixed, never negotiable, so the pane beside it
/// starts in the same place on all eight sections. That fixity is most of
/// what makes the navigation model legible (#1179).
pub const NAV_WIDTH: i32 = 214;

/// How tall the body (sidebar plus pane) is at its smallest.
pub const BODY_HEIGHT: i32 = 330;

/// How far a pane's content sits in from the frame. One number, applied by
/// `.postio-settings-pane-body` in CSS and by the two panes that build a
/// column by hand.
const PANE_INSET: i32 = 22;

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
/// which a keystroke carries no default for. `CommandId::RemoveAccount`,
/// `CommandId::UpdateCredential` (#471), `CommandId::RebuildAccountIndex`
/// (#981) and `CommandId::SetDefaultAccount` (#960) reach the keyboard path
/// by resolving
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
    /// Rebuild this account's local search index (#981).
    RebuildIndex,
    /// Make this the account new messages come from (#960).
    SetDefault,
}

/// What to call when an account row's context menu picks an action.
/// Who to ask to run a `CommandId` a settings button stands for.
type CommandHandler = Box<dyn Fn(CommandId)>;

type AccountActionHandler = Box<dyn Fn(AccountId, AccountAction)>;

/// What to call when an account row's enabled switch is flipped by hand —
/// never fired for the initial state [`SettingsPanel::set_accounts`] sets.
type AccountEnabledHandler = Box<dyn Fn(AccountId, bool)>;

/// One field of the account detail view (#880) committed to a new value.
///
/// An account is database state, not `config.toml` preference (ADR 0005
/// Q6b), so this panel cannot patch a buffer the way [`Section::Filters`]
/// and [`Section::Appearance`] do — it only reports what changed, the same split
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

/// Who to tell when a signature is written or removed (#1086).
type SignatureSavedHandler = Box<dyn Fn(AccountId, &SignatureDraft)>;
type SignatureDeletedHandler = Box<dyn Fn(AccountId, SignatureId)>;

/// A signature as the editor has it: what was typed, and which one it is.
///
/// `id` is `None` for one that does not exist yet — the store assigns it, the
/// same way `AccountRepository::create` assigns an account's. Reporting a new
/// signature with an id would make saving an edit create a second one, which
/// is the bug this distinction exists to prevent.
///
/// No `html`. `Signature` carries an optional rich variant and the composer
/// uses it when there is one, but a rich editor is the composer's formatting
/// toolbar's problem (#339) rather than this form's — so this creates
/// text-only signatures and leaves `html` exactly as it is today, `None`
/// everywhere and correctly handled, rather than shipping a second half of an
/// editor (#1086).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureDraft {
    /// Which signature this is, or `None` for one being created.
    pub id: Option<SignatureId>,
    /// What the picker will show.
    pub name: String,
    /// The signature itself.
    pub text: String,
}

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
    /// One row per account, and the form for the selected one.
    Accounts,
    /// `[filters]` — named saved queries.
    Filters,
    /// `[compose]` — signatures, and where one goes above a quote.
    Composing,
    /// `[ui]` — theme, row density, what the message list shows.
    Appearance,
    /// `[keys]` — command id to binding.
    Keyboard,
    /// `[sync]` — IDLE, polling, what is fetched and when.
    Sync,
    /// The remote-image allow-list (#871) and what has been unsubscribed
    /// from — never a `config.toml` table at all, unlike every other pane
    /// here: it is view state, kept in its own `$XDG_STATE_HOME` key-file
    /// (see [`crate::reader::RemoteImageAllowList`]'s own module doc).
    Privacy,
    /// The file itself, as text — the raw `TextView` every pane used to
    /// share. It is not a fallback: `config.toml` *is* the settings store,
    /// and a pane that shows it whole is how a person reaches a key no form
    /// has grown a control for yet.
    ConfigFile,
}

/// Which heading a pane sits under in the sidebar.
///
/// Two groups, because the drawing has two and because the split is real:
/// `Mail` is about the accounts and the messages in them, `Application` is
/// about this program. A person looking for "how big is my index" is not
/// looking under the same heading as one looking for "what is my address".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Accounts, Filters, Composing.
    Mail,
    /// Appearance, Keyboard, Sync & storage, Privacy, Config file.
    Application,
}

impl Group {
    /// Every group, in sidebar order.
    pub const ALL: [Group; 2] = [Group::Mail, Group::Application];

    /// The sidebar heading, already upper-cased.
    ///
    /// Upper here rather than in CSS: GTK's own `text-transform` does not
    /// apply reliably across every face this application loads, and a
    /// kicker that is capitalised in one place and not the next is the
    /// drift `widgets::kicker` exists to stop.
    pub fn label(self) -> &'static str {
        match self {
            Group::Mail => "MAIL",
            Group::Application => "APPLICATION",
        }
    }
}

impl Section {
    /// Every section, in nav order — the drawing's order, grouped.
    pub const ALL: [Section; 8] = [
        Section::Accounts,
        Section::Filters,
        Section::Composing,
        Section::Appearance,
        Section::Keyboard,
        Section::Sync,
        Section::Privacy,
        Section::ConfigFile,
    ];

    /// Which sidebar heading this pane sits under.
    pub fn group(self) -> Group {
        match self {
            Section::Accounts | Section::Filters | Section::Composing => Group::Mail,
            Section::Appearance
            | Section::Keyboard
            | Section::Sync
            | Section::Privacy
            | Section::ConfigFile => Group::Application,
        }
    }

    /// The top-level TOML key this section's headers start with.
    ///
    /// `[accounts]` and `[filters]` never appear as a bare header — every
    /// account and filter is its own dotted table, `[accounts.personal]` —
    /// so matching is by prefix, not by literal line. `Privacy` never
    /// appears at all, the same as `Accounts` since #470: the nav item
    /// stays and points at a structured widget instead of any text.
    fn key(self) -> &'static str {
        match self {
            Section::Appearance => "ui",
            Section::Keyboard => "keys",
            Section::Accounts => "accounts",
            Section::Sync => "sync",
            Section::Filters => "filters",
            Section::Composing => "compose",
            Section::Privacy => "privacy",
            // Not a table: the pane shows every table there is.
            Section::ConfigFile => "",
        }
    }

    /// The nav label, and the pane's own title.
    ///
    /// The pane repeats its sidebar name as its heading on purpose: with one
    /// pane on screen at a time, the title is the only thing that says which
    /// of the eight you are looking at without moving your eyes back to the
    /// sidebar.
    pub fn label(self) -> &'static str {
        match self {
            Section::Accounts => "Accounts",
            Section::Filters => "Filters",
            Section::Composing => "Composing",
            Section::Appearance => "Appearance",
            Section::Keyboard => "Keyboard",
            Section::Sync => "Sync & storage",
            Section::Privacy => "Privacy",
            Section::ConfigFile => "Config file",
        }
    }

    /// The muted leading icon an unselected row wears.
    pub fn icon(self) -> &'static str {
        match self {
            Section::Accounts => "avatar-default-symbolic",
            Section::Filters => "view-list-symbolic",
            Section::Composing => "document-edit-symbolic",
            Section::Appearance => "preferences-desktop-appearance-symbolic",
            Section::Keyboard => "preferences-desktop-keyboard-symbolic",
            Section::Sync => "emblem-synchronizing-symbolic",
            Section::Privacy => "security-high-symbolic",
            Section::ConfigFile => "text-x-generic-symbolic",
        }
    }

    /// The one line under the pane's title, saying what it is for.
    pub fn description(self) -> &'static str {
        match self {
            Section::Accounts => "Every account this installation signs in to.",
            Section::Filters => "Saved searches, and which of them the sidebar shows.",
            Section::Composing => "Signatures, and where one goes when a quote sits under it.",
            Section::Appearance => "How the message list is drawn, and how much of it fits.",
            Section::Keyboard => "Every command and the key that runs it.",
            Section::Sync => "When mail is fetched, and what the local store keeps.",
            Section::Privacy => "What Postio will not do without being asked.",
            Section::ConfigFile => "The whole file, as text. Everything above writes here.",
        }
    }

    /// What this pane is about, for the find-a-setting field.
    ///
    /// The words a person would type looking for something on this pane,
    /// including the ones the pane's own title does not contain — somebody
    /// hunting for "dark mode" is looking for Appearance, which says
    /// neither word. Deliberately not generated from the controls: a pane
    /// gains and loses controls, and a search that silently stopped
    /// matching would be very hard to notice.
    pub fn keywords(self) -> &'static str {
        match self {
            Section::Accounts => "account address imap smtp password oauth signature server remove",
            Section::Filters => "saved search query pinned sidebar filter",
            Section::Composing => "signature reply forward quote compose",
            Section::Appearance => "theme dark light density row height avatars hover font",
            Section::Keyboard => "key binding shortcut rebind keys chord",
            Section::Sync => "sync idle poll interval storage index attachments notify size",
            Section::Privacy => "remote images trackers unsubscribe read receipts connections",
            Section::ConfigFile => "toml file text editor raw",
        }
    }

    /// The `config.toml` table this pane owns, for the footer line.
    ///
    /// `None` for the two panes that own no table: `Privacy` keeps its
    /// state outside the file entirely, and `Config file` is the file.
    pub fn table(self) -> Option<&'static str> {
        match self {
            Section::Accounts => Some("[accounts]"),
            Section::Filters => Some("[filters]"),
            Section::Composing => Some("[compose]"),
            Section::Appearance => Some("[ui]"),
            Section::Keyboard => Some("[keys]"),
            Section::Sync => Some("[sync]"),
            Section::Privacy | Section::ConfigFile => None,
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

/// `300` → `5 min`, `90` → `90s`, `3600` → `1 h`.
///
/// Pure, and tested as such: this is the sentence under the Check-for-mail
/// control, and it is the only thing on that pane that still says what the
/// interval in the file actually is once the spin button is gone.
fn humanize_interval(seconds: u64) -> String {
    match seconds {
        0 => "never".to_owned(),
        s if s % 3600 == 0 => {
            let hours = s / 3600;
            format!("{hours} h")
        }
        s if s % 60 == 0 => {
            let minutes = s / 60;
            format!("{minutes} min")
        }
        s => format!("{s}s"),
    }
}

/// Two columns with a rule between them, and real space either side of it.
///
/// The space is the point. A divider with columns flush against it reads as
/// a border on two boxes rather than as one pane divided in two, which is
/// what the drawing asks for and what the old flat column could not express
/// at all.
fn two_columns(left: &impl IsA<gtk::Widget>, right: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("postio-settings-columns");

    let left_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left_box.add_css_class("postio-settings-column");
    left_box.set_hexpand(true);
    left_box.append(left);

    let right_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right_box.add_css_class("postio-settings-column");
    right_box.set_hexpand(true);
    right_box.append(right);

    // Equal halves, through a size group rather than `set_homogeneous`:
    // homogeneous shares the width between *every* child, and the third
    // child here is the 1px rule — which duly took a third of the pane and
    // drew itself as a grey block down the middle of it. `hexpand` alone is
    // not enough either, since it only shares out the *extra* space and
    // leaves the wordier column its larger natural width.
    let columns = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    columns.add_widget(&left_box);
    columns.add_widget(&right_box);

    row.append(&left_box);
    row.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    row.append(&right_box);
    row
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
/// the control. Unlike [`SettingsPanel::sync_row`], there is no second
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

/// Appearance's controls, held so the pane can be *updated* from a fresh
/// read of the file rather than rebuilt from one.
///
/// The old panel rebuilt every row on every change, because a `DropDown`
/// being repopulated fires `selected-notify` and there was no way to tell
/// that apart from a person choosing something. `SegmentedControl` and
/// `CheckRow` both know the difference, so the widgets can outlive the
/// value they show.
pub struct AppearanceControls {
    pub theme: SegmentedControl,
    pub density: SegmentedControl,
    /// `26px rows · 41 per screen` — what the density choice above actually
    /// costs, in the units a person is choosing between.
    pub density_stat: gtk::Label,
    pub hover_actions: CheckRow,
    pub key_hints: CheckRow,
    pub sender_avatars: CheckRow,
}

/// Sync & storage's controls, held for the same reason.
pub struct SyncControls {
    pub check_for_mail: SegmentedControl,
    /// What the chosen mode actually means in minutes — the number the
    /// segmented control deliberately does not carry. See
    /// [`SettingsPanel::ensure_sync_controls`] for why the interval is a
    /// fact here rather than a spin button.
    pub interval: gtk::Label,
    /// Three values, so a segmented control rather than the drawing's
    /// checkbox — see [`SettingsPanel::ensure_sync_controls`].
    pub attachments: SegmentedControl,
    pub sync_on_startup: CheckRow,
    pub notify: CheckRow,
    /// `index 38 MB · stores 7.9 GB` and `3 accounts · last pass 41 min`,
    /// the bordered block's two lines.
    pub stats_size: gtk::Label,
    pub stats_accounts: gtk::Label,
    /// Which mailbox roles a notification is worth raising for, as a comma
    /// list (#874). An `Entry`, because it is a list a person types and not
    /// a choice between three things — ADR 0029 Q3.
    pub notify_roles: gtk::Entry,
}

/// Composing's controls.
pub struct ComposingControls {
    pub on_reply: SegmentedControl,
    pub on_forward: SegmentedControl,
}

mod imp {
    use super::*;

    pub struct SettingsPanel {
        /// The window's own header bar: the title, and the find-a-setting
        /// field. Built here rather than by `window.rs` so the search that
        /// filters this sidebar lives beside the sidebar it filters; the
        /// host window mounts it (see [`super::SettingsPanel::header_bar`]).
        pub header_bar: adw::HeaderBar,
        pub search: gtk::SearchEntry,
        /// Which of the eight panes is on screen. Exactly one ever is —
        /// that is the whole navigation model, and the reason this is a
        /// `Stack` and not a column of cards (#1179).
        pub stack: gtk::Stack,
        pub current: Cell<Section>,
        /// The pane's own title and the line under it: the same two strings
        /// the sidebar row carries, repeated where the eye already is.
        pub pane_title: gtk::Label,
        pub pane_description: gtk::Label,
        /// Where a pane's one primary action goes — `Add account` and
        /// nothing else, so far. Empty and invisible on the other seven.
        pub pane_action: gtk::Box,
        /// One row per section, in `Section::ALL` order, so the search
        /// filter and `show_section` can find a row without walking the
        /// list box asking each child what it is.
        pub nav_rows: RefCell<Vec<gtk::ListBoxRow>>,
        /// The footer's state mark: the one thing on the strip that is a
        /// colour rather than a word.
        pub footer_dot: gtk::Box,
        /// Which file is being written, in mono — `[ui] in config.toml` on a
        /// pane that owns a table, the whole path on one that does not.
        pub footer_target: gtk::Label,
        /// What the find-a-setting field currently holds, folded to lower
        /// case — read by the sidebar's filter function, which GTK calls
        /// once per row and must not do the folding eight times over.
        pub nav_query: RefCell<String>,
        /// Who to ask to run a `CommandId` this panel has a button for.
        pub command: RefCell<Vec<CommandHandler>>,
        /// The footer's `Open in $EDITOR` cap, held so a keymap change can
        /// put the live key on it.
        pub editor_button: OnceCell<std::rc::Rc<crate::widgets::KeycapButton>>,
        /// How tall the message list is, so Appearance can say how many rows
        /// of the chosen density fit in one. Zero means nobody has said.
        pub list_viewport: Cell<i32>,
        /// The eight panes themselves.
        pub accounts_pane: gtk::Box,
        pub filters_pane: gtk::Box,
        pub composing_pane: gtk::Box,
        pub appearance_pane: gtk::Box,
        pub keyboard_pane: gtk::Box,
        pub sync_pane: gtk::Box,
        pub privacy_pane: gtk::Box,
        pub config_pane: gtk::Box,
        /// Appearance's controls, built on first draw and updated after —
        /// never rebuilt. Rebuilding was the old panel's way around a
        /// control that reported its own repopulation as a change, and
        /// `SegmentedControl`/`CheckRow` do not have that problem.
        ///
        /// Lazily, though, and for #873's reason: `Window::new` constructs
        /// this panel while it is still wiring its own shortcut controllers,
        /// and building certain controls there was found to corrupt keyboard
        /// routing for the rest of that window's life.
        pub appearance: OnceCell<AppearanceControls>,
        pub sync_controls: OnceCell<SyncControls>,
        pub composing_controls: OnceCell<ComposingControls>,
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
        /// A reindex in progress, per account (#981) — `(done, total)`. An
        /// id absent means nothing is rebuilding that account's index right
        /// now, the same "absent means nothing to say" shape
        /// `token_expiries` uses.
        pub reindex_progress: RefCell<Vec<(AccountId, (u32, u32))>>,
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
        pub account_detail_imap_port: OnceCell<gtk::Entry>,
        pub account_detail_smtp_host: OnceCell<gtk::Entry>,
        pub account_detail_smtp_port: OnceCell<gtk::Entry>,
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
        /// The signature list on the detail view, and the editor it drills
        /// into (#1086). A second level of the same show/hide the account
        /// list and its detail already use.
        pub signature_saved: RefCell<Vec<SignatureSavedHandler>>,
        pub signature_deleted: RefCell<Vec<SignatureDeletedHandler>>,
        pub account_detail_signature_list: OnceCell<gtk::ListBox>,
        pub account_detail_signature_ids_listed: RefCell<Vec<SignatureId>>,
        pub signature_editor: gtk::Box,
        pub signature_editor_name: OnceCell<gtk::Entry>,
        pub signature_editor_text: OnceCell<gtk::TextView>,
        pub signature_editor_error: OnceCell<gtk::Label>,
        pub signature_editor_delete: OnceCell<gtk::Button>,
        /// Which signature the editor is open on, and for which account.
        pub signature_editor_on: RefCell<Option<(AccountId, Option<SignatureId>)>>,
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
                header_bar: adw::HeaderBar::new(),
                search: gtk::SearchEntry::new(),
                stack: gtk::Stack::new(),
                current: Cell::new(Section::Accounts),
                pane_title: gtk::Label::new(None),
                pane_description: gtk::Label::new(None),
                pane_action: gtk::Box::new(gtk::Orientation::Horizontal, 8),
                nav_rows: RefCell::new(Vec::new()),
                footer_dot: gtk::Box::new(gtk::Orientation::Horizontal, 0),
                footer_target: gtk::Label::new(None),
                nav_query: RefCell::new(String::new()),
                command: RefCell::new(Vec::new()),
                editor_button: OnceCell::new(),
                list_viewport: Cell::new(0),
                accounts_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                filters_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                composing_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                appearance_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                keyboard_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                sync_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                privacy_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                config_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                appearance: OnceCell::new(),
                sync_controls: OnceCell::new(),
                composing_controls: OnceCell::new(),
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
                reindex_progress: RefCell::new(Vec::new()),
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
                signature_saved: RefCell::new(Vec::new()),
                signature_deleted: RefCell::new(Vec::new()),
                account_detail_signature_list: OnceCell::new(),
                account_detail_signature_ids_listed: RefCell::new(Vec::new()),
                signature_editor: gtk::Box::new(gtk::Orientation::Vertical, 0),
                signature_editor_name: OnceCell::new(),
                signature_editor_text: OnceCell::new(),
                signature_editor_error: OnceCell::new(),
                signature_editor_delete: OnceCell::new(),
                signature_editor_on: RefCell::new(None),
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
        self.refresh_footer();

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

    /// `account`'s reindex progress right now (#981) — `Some((done, total))`
    /// while `postio_session::reindex_account` is running for it, `None`
    /// once it has finished or nothing is running.
    ///
    /// Replaces any earlier reading for the same account rather than
    /// accumulating one: what a caller reports here is "where the rebuild
    /// is right now", not a log of every step it passed through.
    pub fn set_reindex_progress(&self, account: AccountId, progress: Option<(u32, u32)>) {
        let mut readings = self.imp().reindex_progress.borrow_mut();
        readings.retain(|(id, _)| *id != account);
        if let Some(progress) = progress {
            readings.push((account, progress));
        }
        drop(readings);
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

    /// The reindex line this account's row shows while a rebuild is running
    /// (#981), or `None` when nothing is.
    ///
    /// Said out loud on purpose, not a silent background action: a rebuild
    /// makes search *worse* while it runs — messages drop out of results
    /// until they are reindexed — and a user who pressed the button and saw
    /// nothing on the row would read the silence as the button having done
    /// nothing.
    fn reindex_status(&self, account: AccountId) -> Option<String> {
        let (done, total) = self
            .imp()
            .reindex_progress
            .borrow()
            .iter()
            .find(|(id, _)| *id == account)
            .map(|(_, progress)| *progress)?;
        Some(if total == 0 {
            "Rebuilding search index…".to_owned()
        } else {
            format!("Rebuilding search index — {done} of {total}")
        })
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
        // Selecting a row is what opens the form under the list (#1179), so
        // a redraw that dropped the selection would close a form somebody is
        // typing in. Put it back on the account the form is open on, before
        // anything reads the list's selection back.
        //
        // Copied out of the `RefCell` before selecting, never held across
        // it: `select_row` fires `row-selected` synchronously, which lands
        // in `open_account_detail`, which writes this very cell. Holding the
        // borrow across the call aborts the process — a `RefCell` panic in
        // a GTK signal handler is a panic in a function that cannot unwind.
        let open_on = *imp.account_detail_id.borrow();
        if let Some(open_on) = open_on {
            let mut index = 0;
            while let Some(row) = imp.accounts_list.row_at_index(index) {
                if row_account_id(&row) == open_on {
                    imp.accounts_list.select_row(Some(&row));
                    break;
                }
                index += 1;
            }
        }
        // A refresh can land while the detail view is open on an account
        // this same redraw just found gone -- removed from another window,
        // most likely -- and showing an editable form over settings that no
        // longer exist would let an edit resurrect a deleted account.
        let open_on = *imp.account_detail_id.borrow();
        if let Some(id) = open_on
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
    /// One account: initials, address, the `default` tag, and one mono line
    /// of facts — plus, when the token has expired, the way to fix it.
    ///
    /// The facts used to be four separate labels stacked under the name, one
    /// per thing that had something to say. That reads as four rows of one
    /// account rather than one row of four facts, and the drawing
    /// (`Design/screens/21`) is unambiguous: a name line and a metadata line,
    /// mono, `·`-joined. Nothing is dropped — the pieces are the same
    /// strings, joined.
    fn account_row(&self, account: &Account) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-account-row");
        // Selecting a row is what reveals the form under the list (#1179),
        // so unlike every other list in this panel these rows are
        // selectable.
        row.set_selectable(true);
        // glib cannot know the type a key was stored under; this file can —
        // the same technique `Sidebar`'s rows use for their own ids (#292).
        #[allow(unsafe_code)]
        unsafe {
            row.set_data("postio-account-id", account.id.get());
        }

        let avatar = gtk::Label::new(Some(&crate::row::initials(Some(&account.address))));
        avatar.add_css_class("postio-settings-account-avatar");
        avatar.set_valign(gtk::Align::Center);

        let address = gtk::Label::new(Some(&account.address.address));
        address.add_css_class("postio-settings-account-address");
        address.set_xalign(0.0);
        address.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

        let name_line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        name_line.append(&address);
        // Words, never colour alone -- ADR 0005's own rule for per-account
        // identification. It says what the marker *does*: "primary" would
        // assert a status, and #960's fence is that this account is not more
        // the user's than any other.
        if account.is_default {
            let tag = gtk::Label::new(Some("default"));
            tag.add_css_class("postio-settings-account-default");
            tag.set_valign(gtk::Align::Center);
            tag.set_tooltip_text(Some("New messages come from this account"));
            name_line.append(&tag);
        }

        // One line, `·`-joined, in the order a person reads it: what kind of
        // account, how it signs in, how much mail, and how that stands right
        // now.
        let validity = self.token_validity(account.id);
        let expired = validity
            .as_deref()
            .is_some_and(|text| text.starts_with("token expired"));
        let mut facts = vec![account_badge(account)];
        facts.extend(self.mail_weight(account.id));
        facts.extend(validity.clone());
        facts.extend(self.reindex_status(account.id));
        if !account.enabled {
            facts.push("disabled".to_owned());
        }
        let facts = facts.join(" · ");
        let metadata = stat_line(&facts);
        metadata.add_css_class("postio-settings-account-metadata");

        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        text.append(&name_line);
        // An expired token is the one state on this row that is a problem
        // rather than a fact, so it gets the mark that says so — beside the
        // line it is about, not as a fifth line of its own.
        if expired {
            let flagged = gtk::Box::new(gtk::Orientation::Horizontal, 7);
            let warning = gtk::Image::from_icon_name("dialog-warning-symbolic");
            warning.add_css_class("postio-settings-account-warning");
            flagged.append(&warning);
            flagged.append(&metadata);
            text.append(&flagged);
        } else {
            text.append(&metadata);
        }

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        box_.add_css_class("postio-settings-account-line");
        box_.append(&avatar);
        box_.append(&text);

        // The repair, on the row that needs it. A person whose token has
        // expired is not looking for a context menu; they are looking for
        // the button that fixes it, and it belongs where the problem is
        // stated rather than three keystrokes away.
        if expired {
            let reconnect = gtk::Button::with_label("Reconnect");
            reconnect.add_css_class("postio-settings-small-button");
            reconnect.set_valign(gtk::Align::Center);
            let account_id = account.id;
            reconnect.connect_clicked(glib::clone!(
                #[weak(rename_to = panel)]
                self,
                move |_| panel.request_account_action(account_id, AccountAction::UpdateCredential)
            ));
            box_.append(&reconnect);
        }

        let enabled = gtk::Switch::new();
        enabled.set_active(account.enabled);
        enabled.set_valign(gtk::Align::Center);
        // The one legitimate switch left in this window (ADR 0029 Q2): it
        // does something when flipped — connects or disconnects the account
        // — rather than writing a value into a form.
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
        box_.append(&enabled);

        row.set_child(Some(&box_));
        // The row is announced as a unit, so every line has to be part of
        // the announcement or a screen reader never reaches it.
        let mut announcement = format!("{}, {}", account.display_name, account.address.address);
        if account.is_default {
            announcement.push_str(", default");
        }
        announcement.push_str(&format!(", {facts}"));
        row.update_property(&[gtk::accessible::Property::Label(&announcement)]);
        row
    }

    /// The account list itself, for the focus controller `Window` puts on
    /// it — `Context::Accounts` follows the keyboard into this list and no
    /// further, so the raw `config.toml` view never enters it (ADR 0005
    /// Q6c).
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
            .set_text(&account.incoming.port.to_string());
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
            .set_text(&account.outgoing.port.to_string());
        imp.account_detail_loading.set(false);
        *imp.account_detail_id.borrow_mut() = Some(id);

        // The account's signatures, rebuilt from what it carries (#1086).
        // Rebuilt rather than patched for the reason `redraw_filters` is:
        // the list is small, and a diff is a second description of the same
        // state free to disagree with the first.
        if let Some(list) = imp.account_detail_signature_list.get() {
            while let Some(row) = list.row_at_index(0) {
                list.remove(&row);
            }
            for signature in &account.signatures {
                let row = gtk::ListBoxRow::new();
                let label = gtk::Label::new(Some(&signature.name));
                label.set_xalign(0.0);
                label.add_css_class("postio-settings-signature-row");
                row.set_child(Some(&label));
                row.set_activatable(true);
                row.update_property(&[gtk::accessible::Property::Label(&format!(
                    "Edit the signature {}",
                    signature.name
                ))]);
                list.append(&row);
            }
            *imp.account_detail_signature_ids_listed.borrow_mut() =
                account.signatures.iter().map(|s| s.id).collect();
            // "Empty is never blank" -- but the Add button below says what to
            // do about it, so the list simply goes away rather than drawing a
            // frame around nothing.
            list.set_visible(!account.signatures.is_empty());
        }

        imp.account_detail.set_visible(true);
        // Reopening the account is how the app returns from a save, so the
        // editor must not be left over it.
        imp.signature_editor.set_visible(false);
        *imp.signature_editor_on.borrow_mut() = None;
        // The list stays. Selecting a row reveals the form *under* it
        // (#1179, Design/screens/21) rather than drilling into it, so the
        // account being edited is still on screen with the others — which
        // is what makes moving between accounts one click instead of two.
        imp.accounts_scroller
            .set_visible(!imp.accounts.borrow().is_empty());
    }

    /// Builds the detail view's five field widgets, the first time any
    /// account's detail is opened — never during `build()` or this
    /// widget's own construction.
    ///
    /// `SettingsPanel` is built as a hidden overlay child while `Window::new`
    /// is still wiring up its own overlay siblings and shortcut controllers
    /// (`window.rs`), and constructing a widget with its own internal event
    /// controllers there was found to corrupt keyboard routing for the rest
    /// of that window (#873, about a `gtk::DropDown`) — `gtk::Entry`
    /// carries the same kind of internal `GtkText`
    /// key/IM controllers a `DropDown`'s type-ahead does, so it gets the
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

        // An `Entry`, not a `SpinButton`. A port is a number a person
        // types — 993, 587 — and stepping to it one of 65,535 at a time is
        // not a thing anybody does; the drawing has no spin button anywhere
        // and neither does this window any more (#1179, ADR 0029 Q3).
        let imap_port = gtk::Entry::new();
        imap_port.add_css_class("postio-settings-account-detail-imap-port");
        imap_port.set_input_purpose(gtk::InputPurpose::Digits);
        imap_port.set_max_width_chars(6);
        imap_port.set_halign(gtk::Align::Start);
        imap_port.update_property(&[gtk::accessible::Property::Label("IMAP port")]);
        imap_port.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| {
                // Committed only when it parses. A half-typed `9` on the way
                // to `993` is not a port anybody asked to connect to, and
                // writing it would dial one.
                if let Ok(port) = entry.text().trim().parse::<u16>()
                    && port > 0
                {
                    panel.commit_account_edit(AccountEdit::ImapPort(port));
                }
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

        // An `Entry`, not a `SpinButton`. A port is a number a person
        // types — 993, 587 — and stepping to it one of 65,535 at a time is
        // not a thing anybody does; the drawing has no spin button anywhere
        // and neither does this window any more (#1179, ADR 0029 Q3).
        let smtp_port = gtk::Entry::new();
        smtp_port.add_css_class("postio-settings-account-detail-smtp-port");
        smtp_port.set_input_purpose(gtk::InputPurpose::Digits);
        smtp_port.set_max_width_chars(6);
        smtp_port.set_halign(gtk::Align::Start);
        smtp_port.update_property(&[gtk::accessible::Property::Label("SMTP port")]);
        smtp_port.connect_activate(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| {
                // Committed only when it parses. A half-typed `9` on the way
                // to `993` is not a port anybody asked to connect to, and
                // writing it would dial one.
                if let Ok(port) = entry.text().trim().parse::<u16>()
                    && port > 0
                {
                    panel.commit_account_edit(AccountEdit::SmtpPort(port));
                }
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

        // The account's signatures, and the way to make one (#1086). Every
        // layer under this existed and worked; nothing could feed it, so
        // both this list and the composer's picker showed nothing for ever.
        let signatures = gtk::ListBox::new();
        signatures.add_css_class("postio-settings-signature-list");
        signatures.set_selection_mode(gtk::SelectionMode::None);
        signatures.connect_row_activated(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                let index = row.index();
                let id = panel
                    .imp()
                    .account_detail_signature_ids_listed
                    .borrow()
                    .get(index as usize)
                    .copied();
                if let Some(id) = id {
                    panel.open_signature_editor(Some(id));
                }
            }
        ));
        imp.account_detail
            .append(&detail_row("Signatures", &signatures));
        let _ = imp.account_detail_signature_list.set(signatures);

        let add = gtk::Button::with_label("Add signature");
        add.add_css_class("postio-settings-signature-add");
        add.set_halign(gtk::Align::Start);
        add.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.open_signature_editor(None)
        ));
        imp.account_detail.append(&add);

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

        self.build_signature_editor();
    }

    /// The signature editor: a name, a body, and the two verbs that need
    /// somewhere to live (#1086).
    ///
    /// A second drill-in rather than more rows on the detail view. A
    /// signature is a name *and* a body — a two-field form — and the detail
    /// view is deliberately one control per line; folding a multi-line text
    /// box into that row rhythm would make both harder to read. The panel
    /// already drills in once, so this is the same show/hide one level down.
    fn build_signature_editor(&self) {
        let imp = self.imp();
        if imp.signature_editor_name.get().is_some() {
            return;
        }

        let back = gtk::Button::from_icon_name("go-previous-symbolic");
        back.add_css_class("postio-settings-signature-back");
        back.add_css_class("flat");
        back.set_halign(gtk::Align::Start);
        back.set_tooltip_text(Some("Back to the account"));
        back.update_property(&[gtk::accessible::Property::Label("Back to the account")]);
        back.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.close_signature_editor()
        ));
        imp.signature_editor.append(&back);

        let name = gtk::Entry::new();
        name.add_css_class("postio-settings-signature-name");
        name.update_property(&[gtk::accessible::Property::Label("Signature name")]);
        imp.signature_editor.append(&detail_row("Name", &name));
        let _ = imp.signature_editor_name.set(name);

        let text = gtk::TextView::new();
        text.add_css_class("postio-settings-signature-text");
        text.set_wrap_mode(gtk::WrapMode::WordChar);
        text.update_property(&[gtk::accessible::Property::Label("Signature text")]);
        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&text));
        scroller.set_min_content_height(120);
        imp.signature_editor
            .append(&detail_row("Signature", &scroller));
        let _ = imp.signature_editor_text.set(text);

        // Why a save was refused -- a name already taken, so far. Hidden
        // until there is something to say, and never a raw store error:
        // "UNIQUE constraint failed" is not an answer anybody can act on.
        let error = gtk::Label::new(None);
        error.add_css_class("postio-settings-signature-error");
        error.set_xalign(0.0);
        error.set_wrap(true);
        error.set_visible(false);
        imp.signature_editor.append(&error);
        let _ = imp.signature_editor_error.set(error);

        let verbs = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        verbs.set_halign(gtk::Align::Start);
        let save = gtk::Button::with_label("Save");
        save.add_css_class("postio-settings-signature-save");
        save.add_css_class("suggested-action");
        save.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.save_signature()
        ));
        verbs.append(&save);

        let delete = gtk::Button::with_label("Delete");
        delete.add_css_class("postio-settings-signature-delete");
        delete.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.delete_signature()
        ));
        verbs.append(&delete);
        let _ = imp.signature_editor_delete.set(delete);
        imp.signature_editor.append(&verbs);
    }

    /// Hides the form, leaving the list.
    ///
    /// Not a "back": the list never went away (`open_account_detail`). This
    /// is what runs when the selection is cleared, and when a refresh finds
    /// the account the form was open on gone.
    pub fn close_account_detail(&self) {
        let imp = self.imp();
        *imp.account_detail_id.borrow_mut() = None;
        imp.account_detail.set_visible(false);
        imp.accounts_scroller
            .set_visible(!imp.accounts.borrow().is_empty());
    }

    /// Called when a field in the account detail view is committed —
    /// `Enter` in an `Entry`, or any change to a `SpinButton`.
    /// Called when a signature is saved, with the account and what was typed
    /// (#1086).
    ///
    /// The panel writes nothing, the same split every other edit here uses:
    /// this layer may not link SQLite. `postio-app` persists it and hands
    /// back either a refreshed account list or, when the store refused,
    /// [`set_signature_error`](Self::set_signature_error).
    pub fn connect_signature_saved(&self, handler: impl Fn(AccountId, &SignatureDraft) + 'static) {
        self.imp()
            .signature_saved
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Called when a signature is deleted.
    pub fn connect_signature_deleted(&self, handler: impl Fn(AccountId, SignatureId) + 'static) {
        self.imp()
            .signature_deleted
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Open the signature editor on `id`, or on a new signature.
    pub fn open_signature_editor(&self, id: Option<SignatureId>) {
        let imp = self.imp();
        let Some(account) = *imp.account_detail_id.borrow() else {
            return;
        };
        self.build_signature_editor();
        let signature = id.and_then(|id| {
            imp.accounts
                .borrow()
                .iter()
                .find(|candidate| candidate.id == account)
                .and_then(|candidate| {
                    candidate
                        .signatures
                        .iter()
                        .find(|signature| signature.id == id)
                        .cloned()
                })
        });
        let name = imp.signature_editor_name.get().expect("built above");
        let text = imp.signature_editor_text.get().expect("built above");
        name.set_text(signature.as_ref().map_or("", |s| s.name.as_str()));
        text.buffer()
            .set_text(signature.as_ref().map_or("", |s| s.text.as_str()));
        // Nothing to delete on one that does not exist yet.
        imp.signature_editor_delete
            .get()
            .expect("built above")
            .set_visible(id.is_some());
        self.set_signature_error(None);
        *imp.signature_editor_on.borrow_mut() = Some((account, id));
        imp.account_detail.set_visible(false);
        imp.signature_editor.set_visible(true);
    }

    /// Close the editor and show the account again.
    pub fn close_signature_editor(&self) {
        let imp = self.imp();
        *imp.signature_editor_on.borrow_mut() = None;
        imp.signature_editor.set_visible(false);
        imp.account_detail.set_visible(true);
    }

    /// Say why a save was refused, or clear it.
    ///
    /// The editor stays open on what was typed: a duplicate name is fixed by
    /// changing one word, and throwing the body away to say so would make the
    /// fix cost more than the mistake.
    pub fn set_signature_error(&self, reason: Option<String>) {
        let Some(label) = self.imp().signature_editor_error.get() else {
            return;
        };
        let reason = reason.unwrap_or_default();
        label.set_visible(!reason.is_empty());
        label.set_text(&reason);
        label.update_property(&[gtk::accessible::Property::Label(&reason)]);
    }

    fn save_signature(&self) {
        let Some((account, id)) = *self.imp().signature_editor_on.borrow() else {
            return;
        };
        let imp = self.imp();
        let name = imp.signature_editor_name.get().expect("built").text();
        let buffer = imp.signature_editor_text.get().expect("built").buffer();
        let text = buffer
            .text(&buffer.start_iter(), &buffer.end_iter(), false)
            .to_string();
        // An unnamed signature is one the picker cannot offer, so it is
        // refused here rather than by the store: the picker shows the name.
        if name.trim().is_empty() {
            self.set_signature_error(Some("A signature needs a name".to_owned()));
            return;
        }
        let draft = SignatureDraft {
            id,
            name: name.to_string(),
            text,
        };
        for handler in imp.signature_saved.borrow().iter() {
            handler(account, &draft);
        }
    }

    fn delete_signature(&self) {
        let Some((account, Some(id))) = *self.imp().signature_editor_on.borrow() else {
            return;
        };
        for handler in self.imp().signature_deleted.borrow().iter() {
            handler(account, id);
        }
    }

    /// Press "Add signature". For tests.
    #[doc(hidden)]
    pub fn test_press_add_signature(&self) -> bool {
        if self.imp().account_detail_id.borrow().is_none() {
            return false;
        }
        self.open_signature_editor(None);
        true
    }

    /// Open the editor on a listed signature, as activating its row does.
    #[doc(hidden)]
    pub fn test_open_signature(&self, id: SignatureId) -> bool {
        if !self
            .imp()
            .account_detail_signature_ids_listed
            .borrow()
            .contains(&id)
        {
            return false;
        }
        self.open_signature_editor(Some(id));
        true
    }

    /// Type into the editor. For tests.
    #[doc(hidden)]
    pub fn test_type_signature(&self, name: &str, text: &str) {
        let imp = self.imp();
        if let Some(entry) = imp.signature_editor_name.get() {
            entry.set_text(name);
        }
        if let Some(view) = imp.signature_editor_text.get() {
            view.buffer().set_text(text);
        }
    }

    /// What the editor's name field holds. For tests.
    #[doc(hidden)]
    pub fn test_signature_name(&self) -> String {
        self.imp()
            .signature_editor_name
            .get()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default()
    }

    /// What the editor is refusing to save, if anything. For tests.
    #[doc(hidden)]
    pub fn test_signature_error_text(&self) -> String {
        self.imp()
            .signature_editor_error
            .get()
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }

    /// Press Save. For tests.
    #[doc(hidden)]
    pub fn test_press_save_signature(&self) -> bool {
        let open = self.imp().signature_editor_on.borrow().is_some();
        if open {
            self.save_signature();
        }
        open
    }

    /// Press Delete. For tests.
    #[doc(hidden)]
    pub fn test_press_delete_signature(&self) -> bool {
        let deletable = matches!(*self.imp().signature_editor_on.borrow(), Some((_, Some(_))));
        if deletable {
            self.delete_signature();
        }
        deletable
    }

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
        menu.append(Some("Rebuild search index"), Some("account.rebuild-index"));
        // The registry's own title, so the menu, the palette and the cheat
        // sheet say the same words (#960).
        menu.append(Some("Set as default account"), Some("account.set-default"));
        menu.append(Some("Remove"), Some("account.remove"));

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&imp.accounts_list);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        let actions = gtk::gio::SimpleActionGroup::new();
        for (name, action) in [
            ("update-credential", AccountAction::UpdateCredential),
            ("rebuild-index", AccountAction::RebuildIndex),
            ("set-default", AccountAction::SetDefault),
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
    /// Draws Sync & storage from the buffer's current `[sync]`.
    ///
    /// Same build-once-then-update shape as
    /// [`redraw_ui`](Self::redraw_ui).
    fn redraw_sync(&self) {
        // Controls first — see `redraw_ui` for why.
        let controls = self.ensure_sync_controls();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        controls
            .check_for_mail
            .set_selected(match config.sync.check_for_mail {
                CheckForMail::Idle => 0,
                CheckForMail::Poll => 1,
                CheckForMail::Manual => 2,
            });
        controls
            .attachments
            .set_selected(match config.sync.attachment_fetch {
                AttachmentFetch::OnOpen => 0,
                AttachmentFetch::Eager => 1,
                AttachmentFetch::Never => 2,
            });
        controls
            .sync_on_startup
            .set_active(config.sync.sync_on_startup);
        controls.notify.set_active(config.sync.notify);
        // Set before anything reads it back: an `Entry` has no silent
        // setter of its own, and `changed` is not what commits this field —
        // `activate` is, so a redraw cannot write anything.
        controls
            .notify_roles
            .set_text(&config.sync.notify_roles.join(", "));
        let interval = humanize_interval(config.sync.poll_interval_secs);
        controls
            .interval
            .set_label(&match config.sync.check_for_mail {
                CheckForMail::Idle => format!("push · {interval} as a backstop"),
                CheckForMail::Poll => format!("every {interval}"),
                CheckForMail::Manual => "only when you ask".to_owned(),
            });
        self.refresh_storage_stats();
    }

    /// The bordered block's two lines, from the footprints `window.rs` has
    /// measured.
    ///
    /// **An incomplete header pass makes every figure a lower bound**, and
    /// [`postio_core::event::MailFootprint::complete`] is what says so — a
    /// total that silently climbs every few seconds reads as a bug, so the
    /// line says `over 1.4 GB` until the pass finishes rather than showing a
    /// number it will contradict in a moment.
    fn refresh_storage_stats(&self) {
        let imp = self.imp();
        let Some(controls) = imp.sync_controls.get() else {
            return;
        };
        let weights = imp.weights.borrow();
        let local: u64 = weights.iter().map(|(_, w)| w.local_bytes).sum();
        let total: u64 = weights.iter().map(|(_, w)| w.total_bytes).sum();
        let complete = weights.iter().all(|(_, w)| w.complete);
        controls.stats_size.set_label(&if weights.is_empty() {
            "stores not measured yet".to_owned()
        } else {
            format!(
                "stored {}\nknown {}",
                postio_ui::format::human_size_bound(local, complete),
                postio_ui::format::human_size_bound(total, complete)
            )
        });

        let accounts = imp.accounts.borrow().len();
        controls.stats_accounts.set_label(&format!(
            "{accounts} account{}",
            if accounts == 1 { "" } else { "s" }
        ));
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

    /// Builds Sync & storage's controls once — see
    /// [`ensure_appearance`](Self::ensure_appearance) for why lazily.
    ///
    /// # Where the poll interval went
    ///
    /// The middle segment says `Every 5 min` and sets both
    /// `check_for_mail = "poll"` and `poll_interval_secs = 300`, where the
    /// old pane had a `SpinButton` for the seconds. That is deliberate: the
    /// drawing has no spin button anywhere (#1179), and an interval in
    /// seconds is not a choice between three things — it is a number, and
    /// the pane for typing a number into this file is `Config file`, which
    /// the footer names from every pane. Somebody who wants ninety seconds
    /// still has `[sync] poll_interval_secs`, and this control shows them
    /// what they have chosen rather than rounding it away: an interval that
    /// is not five minutes still selects this segment, and the stat line
    /// under it says what the interval actually is.
    fn ensure_sync_controls(&self) -> &SyncControls {
        let imp = self.imp();
        if let Some(controls) = imp.sync_controls.get() {
            return controls;
        }

        let check_for_mail =
            SegmentedControl::new("Check for mail", &["IMAP IDLE", "Every 5 min", "Manual"]);
        check_for_mail.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                panel.apply_sync_mutation(move |sync| match index {
                    1 => {
                        // Only when this is a *move* to polling: choosing
                        // the segment that is already chosen must not
                        // overwrite an interval somebody set on purpose.
                        if sync.check_for_mail != CheckForMail::Poll {
                            sync.poll_interval_secs = POLL_EVERY_FIVE_MINUTES;
                        }
                        sync.check_for_mail = CheckForMail::Poll;
                    }
                    2 => sync.check_for_mail = CheckForMail::Manual,
                    _ => sync.check_for_mail = CheckForMail::Idle,
                })
            }
        ));

        // The drawing draws this as a checkbox, and it cannot be one:
        // `attachment_fetch` has three values, and a checkbox that can only
        // say two of them would silently rewrite `never` to `eager` the
        // first time anybody touched it. Three closed options is exactly
        // what a segmented control is for (ADR 0029 Q1).
        let attachments =
            SegmentedControl::new("Download attachments", &["When opened", "Always", "Never"]);
        attachments.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                let fetch = match index {
                    1 => AttachmentFetch::Eager,
                    2 => AttachmentFetch::Never,
                    _ => AttachmentFetch::OnOpen,
                };
                panel.apply_sync_mutation(move |sync| sync.attachment_fetch = fetch);
            }
        ));
        let sync_on_startup = CheckRow::new("Check for mail when Postio starts");
        sync_on_startup.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |active| panel.apply_sync_mutation(move |sync| sync.sync_on_startup = active)
        ));
        let notify = CheckRow::new("Notify about new mail");
        notify.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |active| panel.apply_sync_mutation(move |sync| sync.notify = active)
        ));

        let notify_roles = gtk::Entry::new();
        notify_roles.add_css_class("postio-settings-notify-roles");
        notify_roles.set_placeholder_text(Some("inbox, flagged"));
        notify_roles.update_property(&[gtk::accessible::Property::Label(
            "Mailbox roles worth a notification, comma separated",
        )]);
        // On `activate`, not on `changed`: this writes to the file, and
        // committing a half-typed role list on every keystroke would put
        // `inbo` in `config.toml` on the way to `inbox`.
        notify_roles.connect_activate(glib::clone!(
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

        let interval = stat_line("");
        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.append(&kicker("CHECK FOR MAIL"));
        check_for_mail.widget().set_margin_top(8);
        left.append(check_for_mail.widget());
        interval.set_margin_top(10);
        left.append(&interval);
        let attachments_kicker = kicker("DOWNLOAD ATTACHMENTS");
        attachments_kicker.set_margin_top(20);
        left.append(&attachments_kicker);
        attachments.widget().set_margin_top(8);
        left.append(attachments.widget());

        let checks = gtk::Box::new(gtk::Orientation::Vertical, 6);
        checks.set_margin_top(20);
        checks.append(sync_on_startup.widget());
        checks.append(notify.widget());
        left.append(&checks);
        let roles_kicker = kicker("NOTIFY FOR");
        roles_kicker.set_margin_top(18);
        left.append(&roles_kicker);
        notify_roles.set_margin_top(8);
        notify_roles.set_halign(gtk::Align::Start);
        notify_roles.set_width_chars(24);
        left.append(&notify_roles);
        let elsewhere = stat_line("remote images are allowed per sender, under Privacy");
        // Wraps rather than ellipsising: it is a sentence, not a column of
        // numbers, and half of it is worse than two lines of it.
        elsewhere.set_ellipsize(pango::EllipsizeMode::None);
        elsewhere.set_wrap(true);
        elsewhere.set_margin_top(18);
        left.append(&elsewhere);

        // The stat block: bordered, mono, with the one action that has a
        // command behind it. `Compact index` is in the drawing and is *not*
        // here — no command compacts an index, and a button wired to
        // nothing is worse than no button. Rebuilding one account's index
        // is on that account's own row, which is where it can name what it
        // is rebuilding.
        let stats_size = stat_line("");
        let stats_accounts = stat_line("");
        let stats = gtk::Box::new(gtk::Orientation::Vertical, 6);
        stats.add_css_class("postio-stat-block");
        stats.append(&stats_size);
        stats.append(&stats_accounts);

        let sync_now = gtk::Button::with_label("Sync now");
        sync_now.add_css_class("postio-settings-small-button");
        sync_now.set_halign(gtk::Align::Start);
        sync_now.set_margin_top(6);
        sync_now.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.request_command(CommandId::Refresh)
        ));
        stats.append(&sync_now);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 0);
        right.append(&kicker("LOCAL STORE"));
        stats.set_margin_top(8);
        right.append(&stats);

        imp.sync_pane.append(&two_columns(&left, &right));

        let _ = imp.sync_controls.set(SyncControls {
            check_for_mail,
            interval,
            attachments,
            sync_on_startup,
            notify,
            stats_size,
            stats_accounts,
            notify_roles,
        });
        imp.sync_controls.get().expect("just set")
    }

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
        // The keycap *is* the control (Design/screens/22): pressing the key
        // a command is bound to is what you press to change it, and while
        // it is waiting the cap says `press a key…` in place of the key it
        // is about to lose. A separate `Rebind` button beside a cap that
        // was only a label made the row two lines and the verb ambiguous —
        // it read as though the cap and the button did different things.
        let rebind = gtk::Button::with_label(if capturing {
            "press a key…"
        } else {
            current.as_str()
        });
        rebind.add_css_class("postio-settings-keys-binding");
        rebind.set_valign(gtk::Align::Center);
        if capturing {
            rebind.add_css_class("postio-settings-keys-capturing");
        }
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

        // One line: what it does on the left, the key on the right. The
        // conflict message is the only thing that ever adds a second, and
        // only on the row being rebound.
        let lines = gtk::Box::new(gtk::Orientation::Vertical, 2);
        lines.set_hexpand(true);
        lines.append(&title);
        lines.append(&conflict);

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        box_.add_css_class("postio-settings-keys-line");
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
    /// Draws Appearance from the buffer's current `[ui]`.
    ///
    /// Builds the controls the first time and only *updates* them after —
    /// the old pane rebuilt every row on every change, because a `DropDown`
    /// being repopulated fires `selected-notify` and there was no telling
    /// that apart from a person choosing something.
    /// [`SegmentedControl::set_selected`] and [`CheckRow::set_active`] both
    /// know the difference (#1179).
    fn redraw_ui(&self) {
        // The controls come first and unconditionally. A file that does not
        // parse is a file whose *values* cannot be read — it is not a
        // reason for this pane to have nothing in it, and one unknown
        // variant three tables away used to empty the whole thing while the
        // footer, correctly, explained why (#1179).
        let controls = self.ensure_appearance();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        controls.theme.set_selected(match config.ui.theme {
            Theme::System => 0,
            Theme::Light => 1,
            Theme::Dark => 2,
        });
        controls.density.set_selected(match config.ui.density {
            Density::Airy => 0,
            Density::Comfortable => 1,
            Density::Compact => 2,
        });
        controls
            .density_stat
            .set_label(&self.density_stat_text(config.ui.density));
        controls
            .hover_actions
            .set_active(config.ui.show_hover_actions);
        controls.key_hints.set_active(config.ui.show_key_hints);
        controls.sender_avatars.set_active(config.ui.sender_avatars);
    }

    /// What the chosen density actually costs, in the units a person is
    /// choosing between: `40px rows · 18 per screen`.
    ///
    /// **Measured, not tabulated.** The height comes from a real
    /// [`crate::row::MessageRowView`] laid out at this density with a
    /// representative message in it, because that is the only number that
    /// stays true when the row's anatomy or the font changes; a constant
    /// here would be a second source of truth that nothing keeps in step.
    ///
    /// The per-screen figure needs the height of the list the rows go in,
    /// which this widget cannot see — `window.rs` hands it over
    /// ([`SettingsPanel::set_list_viewport_height`]). Without it the line
    /// says only what it knows, rather than dividing by a guess.
    fn density_stat_text(&self, density: Density) -> String {
        let probe = crate::row::MessageRowView::new();
        probe.set_density(density);
        probe.set_row(Some(crate::list::Row {
            id: postio_model::ids::MessageId::new(1),
            thread: None,
            from: Some(postio_model::EmailAddress::new(
                Some("Ada Lovelace"),
                "ada@example.com",
            )),
            subject: Some("A representative subject line".into()),
            preview: Some("And the snippet under it, which the compact density drops.".into()),
            received_at: chrono::Utc::now(),
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            has_attachments: false,
            thread_count: 1,
            participants: Vec::new(),
        }));
        let height = probe.measured_height(DENSITY_PROBE_WIDTH).ceil() as i32;
        let height = height.max(1);
        match self.imp().list_viewport.get() {
            viewport if viewport > 0 => {
                format!("{height}px rows · {} per screen", viewport / height)
            }
            _ => format!("{height}px rows"),
        }
    }

    /// How tall the message list is, so the density line can say how many
    /// rows fit in it. Zero means "not known yet", and the line says less
    /// rather than guessing.
    pub fn set_list_viewport_height(&self, height: i32) {
        self.imp().list_viewport.set(height);
        if let Some(controls) = self.imp().appearance.get() {
            let density = Config::from_toml_str(&self.text())
                .map(|config| config.ui.density)
                .unwrap_or_default();
            controls
                .density_stat
                .set_label(&self.density_stat_text(density));
        }
    }

    /// Builds Appearance's controls once, and returns them thereafter.
    ///
    /// Lazily, and for #873's reason: `Window::new` constructs this panel
    /// while it is still wiring its own shortcut controllers, and building
    /// certain controls in that window turned out to corrupt keyboard
    /// routing for the rest of its life. Every pane here populates on first
    /// draw instead, which is after that construction has finished.
    fn ensure_appearance(&self) -> &AppearanceControls {
        let imp = self.imp();
        if let Some(controls) = imp.appearance.get() {
            return controls;
        }

        let theme = SegmentedControl::new("Theme", &["System", "Light", "Dark"]);
        theme.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                let theme = match index {
                    1 => Theme::Light,
                    2 => Theme::Dark,
                    _ => Theme::System,
                };
                panel.apply_ui_mutation(move |ui| ui.theme = theme);
            }
        ));

        let density = SegmentedControl::new("Row density", &["Airy", "Snug", "Compact"]);
        density.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                let density = match index {
                    1 => Density::Comfortable,
                    2 => Density::Compact,
                    _ => Density::Airy,
                };
                panel.apply_ui_mutation(move |ui| ui.density = density);
            }
        ));

        let density_stat = stat_line("");

        let hover_actions = CheckRow::new("Hover action icons");
        hover_actions.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |active| panel.apply_ui_mutation(move |ui| ui.show_hover_actions = active)
        ));
        let key_hints = CheckRow::new("Key hints on the focused row");
        key_hints.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |active| panel.apply_ui_mutation(move |ui| ui.show_key_hints = active)
        ));
        let sender_avatars = CheckRow::new("Sender avatars");
        sender_avatars.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |active| panel.apply_ui_mutation(move |ui| ui.sender_avatars = active)
        ));

        let left = gtk::Box::new(gtk::Orientation::Vertical, 0);
        left.append(&kicker("THEME"));
        theme.widget().set_margin_top(8);
        left.append(theme.widget());
        let density_kicker = kicker("ROW DENSITY");
        density_kicker.set_margin_top(22);
        left.append(&density_kicker);
        density.widget().set_margin_top(8);
        left.append(density.widget());
        density_stat.set_margin_top(10);
        left.append(&density_stat);

        let right = gtk::Box::new(gtk::Orientation::Vertical, 10);
        right.append(&kicker("MESSAGE LIST"));
        let checks = gtk::Box::new(gtk::Orientation::Vertical, 6);
        checks.set_margin_top(4);
        checks.append(hover_actions.widget());
        checks.append(key_hints.widget());
        checks.append(sender_avatars.widget());
        right.append(&checks);

        imp.appearance_pane.append(&two_columns(&left, &right));

        let _ = imp.appearance.set(AppearanceControls {
            theme,
            density,
            density_stat,
            hover_actions,
            key_hints,
            sender_avatars,
        });
        imp.appearance.get().expect("just set")
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

    /// Draws Composing from the buffer's current `[compose]`.
    fn redraw_compose(&self) {
        // Controls first — see `redraw_ui` for why.
        let controls = self.ensure_composing();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        let index = |placement| match placement {
            SignaturePlacement::AboveQuote => 0,
            SignaturePlacement::BelowQuote => 1,
        };
        controls
            .on_reply
            .set_selected(index(config.compose.signature_on_reply));
        controls
            .on_forward
            .set_selected(index(config.compose.signature_on_forward));
    }

    /// Builds Composing's controls once — see
    /// [`ensure_appearance`](Self::ensure_appearance) for why lazily.
    fn ensure_composing(&self) -> &ComposingControls {
        let imp = self.imp();
        if let Some(controls) = imp.composing_controls.get() {
            return controls;
        }

        let placement = |index: usize| match index {
            1 => SignaturePlacement::BelowQuote,
            _ => SignaturePlacement::AboveQuote,
        };
        let options = &["Above the quote", "Below the quote"];

        let on_reply = SegmentedControl::new("Signature on a reply", options);
        on_reply.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                panel.apply_compose_mutation(move |compose| {
                    compose.signature_on_reply = placement(index)
                })
            }
        ));
        let on_forward = SegmentedControl::new("Signature on a forward", options);
        on_forward.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                panel.apply_compose_mutation(move |compose| {
                    compose.signature_on_forward = placement(index)
                })
            }
        ));

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&kicker("SIGNATURE ON A REPLY"));
        on_reply.widget().set_margin_top(8);
        column.append(on_reply.widget());
        let forward_kicker = kicker("SIGNATURE ON A FORWARD");
        forward_kicker.set_margin_top(22);
        column.append(&forward_kicker);
        on_forward.widget().set_margin_top(8);
        column.append(on_forward.widget());
        let note = stat_line("a reply answers a fragment · a forward hands the whole message on");
        note.set_margin_top(14);
        column.append(&note);
        column.set_margin_start(PANE_INSET);
        column.set_margin_end(PANE_INSET);
        column.set_margin_top(PANE_INSET);
        imp.composing_pane.append(&column);

        let _ = imp.composing_controls.set(ComposingControls {
            on_reply,
            on_forward,
        });
        imp.composing_controls.get().expect("just set")
    }

    /// Applies `mutate` to the buffer's `[compose]` table and writes the
    /// result back, the same format-preserving path every other structured
    /// pane uses.
    fn apply_compose_mutation(&self, mutate: impl FnOnce(&mut postio_config::ComposeConfig)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config.compose);
        match patch_compose(&original, &config.compose) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [compose]"),
        }
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
    /// The footer follows the cursor while the file itself is on screen.
    ///
    /// It used to move the sidebar's selection, back when the sidebar was a
    /// table of contents for one long text view. The sidebar picks panes
    /// now, so what the cursor still decides is the one thing it honestly
    /// can: which table the strip along the foot says you are typing in.
    fn sync_nav(&self) {
        if self.imp().current.get() != Section::ConfigFile {
            return;
        }
        self.refresh_footer();
    }

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

    /// The header bar the host window mounts: the title, and the field that
    /// filters the sidebar.
    ///
    /// Built here rather than by `window.rs` because the search filters
    /// *this* sidebar, and a control whose whole behaviour lives in another
    /// module is how the three keycap implementations happened.
    pub fn header_bar(&self) -> adw::HeaderBar {
        self.imp().header_bar.clone()
    }

    /// Which pane is on screen.
    pub fn current_section(&self) -> Section {
        self.imp().current.get()
    }

    /// Shows exactly one pane, and makes every other part of the frame agree
    /// with it.
    ///
    /// This is the navigation model the rebuild exists for (#1179): the
    /// window, its header bar and its footer are identical on all eight
    /// panes, and only the sidebar's selection and the pane's body change.
    /// Everything that has to move when the selection moves moves here, so
    /// there is one place to read to know what a pane switch does.
    pub fn show_section(&self, section: Section) {
        let imp = self.imp();
        let previous = imp.current.replace(section);
        imp.stack.set_visible_child_name(section.label());
        imp.pane_title.set_label(section.label());
        imp.pane_description.set_label(section.description());
        // Only Accounts has a primary action so far. The box stays in the
        // header on every pane rather than being added and removed, so the
        // title does not shift sideways as you move down the sidebar.
        imp.pane_action.set_visible(section == Section::Accounts);

        // Panes that build their controls on first draw (#873) draw here,
        // which is the first moment one of them is actually looked at.
        match section {
            Section::Appearance => self.redraw_ui(),
            Section::Sync => self.redraw_sync(),
            Section::Composing => self.redraw_compose(),
            Section::Keyboard => {
                self.ensure_capture_controller();
                self.redraw_keys();
            }
            // Arriving at the file itself from a pane that owns a table
            // puts the cursor on that table, so "show me the rest of this"
            // lands where the person just was rather than at line one.
            Section::ConfigFile if previous.table().is_some() => self.jump_to(previous),
            _ => {}
        }

        if let Some(row) = Section::ALL
            .iter()
            .position(|candidate| *candidate == section)
            .and_then(|index| imp.nav_rows.borrow().get(index).cloned())
            && !row.is_selected()
        {
            imp.nav.select_row(Some(&row));
        }
        self.refresh_footer();
    }

    /// Puts the raw view's cursor on `section`'s table.
    ///
    /// Only the `Config file` pane has a raw view to move, so this is no
    /// longer navigation — it is what makes arriving at the file from a form
    /// land somewhere useful.
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
    }

    /// Narrows the sidebar to the sections a query matches.
    ///
    /// Matching is over the section's own name and the words its pane is
    /// about, not over the controls themselves: a person typing "dark" wants
    /// to be *taken to* Appearance, and a filter that hid every control but
    /// one would leave them looking at a pane with a hole in it.
    fn apply_search(&self, query: &str) {
        *self.imp().nav_query.borrow_mut() = query.trim().to_lowercase();
        self.imp().nav.invalidate_filter();
    }

    /// Whether `section` survives the current search.
    fn matches_search(&self, section: Section) -> bool {
        let query = self.imp().nav_query.borrow();
        if query.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {}",
            section.label(),
            section.description(),
            section.keywords()
        )
        .to_lowercase();
        query.split_whitespace().all(|word| haystack.contains(word))
    }

    /// Tells the panel who to ask to run a command it has a button for.
    ///
    /// The panel raises `CommandId`s and runs none of them, for the same
    /// reason it never writes an account edit itself: `postio-app` owns the
    /// store and the network, and `window.rs` already has the one dispatch
    /// every keystroke and every menu item goes through. A second path from
    /// a button straight to the runtime is how two surfaces come to disagree
    /// about what `Refresh` means.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().command.borrow_mut().push(Box::new(handler));
    }

    /// Asks whoever is listening to run `command`.
    fn request_command(&self, command: CommandId) {
        for handler in self.imp().command.borrow().iter() {
            handler(command);
        }
    }

    /// Puts the live key on the footer's `Open in $EDITOR` cap.
    ///
    /// The same contract every other keycap in the application keeps: the
    /// key comes from the resolved keymap, so a `[keys]` rebind changes what
    /// the button says the moment it changes what the keyboard does. Called
    /// from `crate::config`, beside the other surfaces that take a keymap.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        if let Some(editor) = self.imp().editor_button.get() {
            editor.set_key(keymap.binding(CommandId::EditConfig));
        }
    }

    /// The footer strip: what is being written, and whether it is valid.
    ///
    /// Present and identical on every pane, which is half of what makes the
    /// navigation model legible — the frame does not move, only the pane
    /// inside it. What changes per pane is one string: the table that pane
    /// owns, or the whole path on the two panes that own none.
    fn refresh_footer(&self) {
        let imp = self.imp();
        let section = imp.current.get();
        // On the file itself the cursor decides, so the strip names the
        // table you are actually typing in rather than the file you already
        // know you are in.
        let table = if section == Section::ConfigFile {
            section_at_line(&self.text(), self.cursor_line()).and_then(Section::table)
        } else {
            section.table()
        };
        let mut target = match (table, imp.path.borrow().as_ref()) {
            (Some(table), _) => format!("{table} in {FILE_NAME}"),
            (None, Some(path)) => display_path(path),
            (None, None) => FILE_NAME.to_owned(),
        };
        // How many keys are not what this build ships with — the one number
        // the Keyboard pane owes, and the drawing puts it here rather than
        // on any row (Design/screens/22).
        if section == Section::Keyboard
            && let Ok(config) = Config::from_toml_str(&self.text())
        {
            let rebound = config.keys.overrides().len();
            if rebound > 0 {
                target.push_str(&format!(" · {rebound} rebound"));
            }
        }
        imp.footer_target.set_label(&target);
    }

    /// What the footer strip says is being written — `[ui] in config.toml`,
    /// or the whole path on a pane that owns no table.
    pub fn footer_target_text(&self) -> String {
        self.imp().footer_target.label().to_string()
    }

    /// Which line the raw view's cursor is on.
    fn cursor_line(&self) -> usize {
        let buffer = &self.imp().buffer;
        let iter = buffer.iter_at_mark(&buffer.get_insert());
        usize::try_from(iter.line()).unwrap_or(0)
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-settings");

        // A dialog to a screen reader: it takes the keyboard and `Escape`
        // closes it, the same contract as the cheat sheet.
        self.set_accessible_role(gtk::AccessibleRole::Dialog);
        self.update_property(&[gtk::accessible::Property::Label("Settings")]);

        self.build_header_bar();

        // ── accounts: one row each, an enable switch, a context menu ─────
        imp.accounts_list
            .add_css_class("postio-settings-accounts-list");
        imp.accounts_list
            .set_selection_mode(gtk::SelectionMode::Single);
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

        // Selecting a row is what reveals the form under the list (#1179):
        // the drawing has one list and one form, not a list you drill out
        // of. Activation (Enter, double click) still opens the same form,
        // so nothing that used to work has stopped.
        imp.accounts_list.set_activate_on_single_click(true);
        imp.accounts_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| match row {
                Some(row) => {
                    let id = row_account_id(row);
                    if id.is_assigned() {
                        panel.open_account_detail(id);
                    }
                }
                None => panel.close_account_detail(),
            }
        ));
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
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Never);
        imp.accounts_scroller.set_propagate_natural_height(true);
        imp.accounts_scroller
            .add_css_class("postio-settings-accounts");
        imp.accounts_scroller.set_visible(false);

        // ── account detail: display name, IMAP/SMTP host+port (#880) ─────
        imp.account_detail
            .add_css_class("postio-settings-account-detail");
        imp.account_detail.set_visible(false);
        imp.signature_editor
            .add_css_class("postio-settings-signature-editor");
        imp.signature_editor.set_visible(false);
        // The five field widgets (Entry/SpinButton) are deliberately NOT
        // built here -- see `ensure_account_detail_fields`'s own doc for
        // why constructing them this early would repeat #873.

        // `Add account` is the pane's one primary action, in the pane
        // header where the drawing puts it — not in the sidebar, which
        // names places rather than verbs.
        let add_account = gtk::Button::with_label("Add account");
        add_account.add_css_class("postio-settings-primary");
        add_account.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.request_command(CommandId::AddAccount)
        ));
        imp.pane_action.append(&add_account);

        // The list and the form share one scrolling column, so a long
        // account list and a long form do not fight over which of them gets
        // the height (Design/screens/21 shows both at once).
        let accounts_column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        accounts_column.append(&imp.accounts_scroller);
        accounts_column.append(&imp.account_detail);
        accounts_column.append(&imp.signature_editor);
        let accounts_scroll = gtk::ScrolledWindow::new();
        accounts_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        accounts_scroll.set_vexpand(true);
        accounts_scroll.set_child(Some(&accounts_column));
        accounts_scroll.update_property(&[gtk::accessible::Property::Label("Accounts")]);
        imp.accounts_pane.append(&accounts_scroll);

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
        imp.filters_scroller.set_vexpand(true);
        imp.filters_scroller
            .add_css_class("postio-settings-filters");
        imp.filters_scroller.set_visible(false);

        imp.filters_empty
            .add_css_class("postio-settings-filters-empty");
        imp.filters_empty.set_xalign(0.0);
        imp.filters_empty.set_wrap(true);
        imp.filters_empty.set_visible(false);

        imp.filters_pane.append(&imp.filters_scroller);
        imp.filters_pane.append(&imp.filters_empty);

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
        // A second list under the same pane as `privacy_list`, so it gets
        // its own heading to tell the two apart — the only pane here that
        // holds two lists.
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

        imp.privacy_pane.append(&kicker("REMOTE IMAGES ALLOWED"));
        imp.privacy_pane.append(&imp.privacy_scroller);
        imp.privacy_pane.append(&imp.privacy_empty);
        let unsubscribe_title = kicker("MAILING LISTS LEFT");
        unsubscribe_title.set_margin_top(18);
        imp.privacy_pane.append(&unsubscribe_title);
        imp.privacy_pane.append(&imp.unsubscribe_scroller);
        imp.privacy_pane.append(&imp.unsubscribe_empty);
        let receipts_title = kicker("READ RECEIPTS");
        receipts_title.set_margin_top(18);
        imp.privacy_pane.append(&receipts_title);
        imp.privacy_pane.append(&imp.read_receipt_count);
        let egress_title = kicker("RECENT CONNECTIONS");
        egress_title.set_margin_top(18);
        imp.privacy_pane.append(&egress_title);
        imp.privacy_pane.append(&imp.egress_scroller);

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
        imp.keys_scroller.set_vexpand(true);
        imp.keys_scroller.add_css_class("postio-settings-keys");
        imp.keys_scroller
            .update_property(&[gtk::accessible::Property::Label("Keybindings")]);

        // `Reset to defaults` is here and `Import mutt bindings` and the
        // Mnemonic/Vim/Emacs set switcher from the drawing are not: this
        // build has one set of defaults and no importer, and a control
        // wired to nothing is worse than a control that is missing.
        let reset_keys = gtk::Button::with_label("Reset to defaults");
        reset_keys.add_css_class("postio-settings-small-button");
        reset_keys.set_halign(gtk::Align::Start);
        reset_keys.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.reset_keys()
        ));
        let keys_actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        keys_actions.add_css_class("postio-settings-pane-actions");
        keys_actions.append(&reset_keys);

        imp.keyboard_pane.append(&imp.keys_scroller);
        imp.keyboard_pane.append(&keys_actions);

        // ── the file itself: the raw view every pane used to share ───────
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

        imp.revert.add_css_class("postio-settings-revert");
        imp.revert.set_halign(gtk::Align::Start);
        imp.revert.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.revert()
        ));
        let config_actions = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        config_actions.add_css_class("postio-settings-pane-actions");
        config_actions.append(&imp.revert);

        imp.config_pane.append(&view_scroller);
        imp.config_pane.append(&config_actions);

        // ── the frame: sidebar, one pane, footer ─────────────────────────
        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.add_css_class("postio-settings-body");
        body.append(&self.build_sidebar());
        body.append(&self.build_pane_area());

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&body);
        column.append(&self.build_footer());
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
                panel.redraw_visible_pane();
                panel.schedule_write();
            }
        ));
        imp.buffer.connect_cursor_position_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.sync_nav()
        ));

        // `Escape` closes it, caught in the capture phase so it works
        // regardless of which child has focus. The same contract
        // `window.rs`'s own controller uses.
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
        // Accounts is where the window opens, per the drawing. Deliberately
        // *not* redraw_ui()/redraw_sync() here: `Window::new` constructs
        // this panel as a hidden child while it is still wiring its own
        // shortcut controllers, and building certain controls mid-
        // construction was found to corrupt keyboard routing for the rest
        // of that window's life -- gtk_finder, gtk_finder_focus,
        // gtk_move_picker and gtk_toggle_sidebar all failed until it was
        // removed (#873). Every pane that builds controls populates from
        // `show_section` instead, which cannot run before the window is up.
        self.show_section(Section::Accounts);
    }

    /// The title and the find-a-setting field.
    fn build_header_bar(&self) {
        let imp = self.imp();
        let title = gtk::Label::new(Some("SETTINGS"));
        title.add_css_class("postio-settings-window-title");
        imp.header_bar.set_title_widget(Some(&title));

        imp.search.set_placeholder_text(Some("Find a setting"));
        imp.search.add_css_class("postio-settings-search");
        imp.search.set_width_chars(24);
        imp.search
            .update_property(&[gtk::accessible::Property::Label("Find a setting")]);
        imp.search.connect_changed(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |entry| panel.apply_search(&entry.text())
        ));
        imp.header_bar.pack_start(&imp.search);
    }

    /// The fixed sidebar: eight sections under two headings.
    fn build_sidebar(&self) -> gtk::ScrolledWindow {
        let imp = self.imp();
        imp.nav.add_css_class("postio-settings-nav-list");
        imp.nav.set_selection_mode(gtk::SelectionMode::Single);
        imp.nav.set_activate_on_single_click(true);

        let mut rows = Vec::with_capacity(Section::ALL.len());
        for section in Section::ALL {
            let row = gtk::ListBoxRow::new();
            row.add_css_class("postio-settings-nav-row");
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let icon = gtk::Image::from_icon_name(section.icon());
            icon.add_css_class("postio-settings-nav-icon");
            line.append(&icon);
            let label = gtk::Label::new(Some(section.label()));
            label.set_xalign(0.0);
            label.set_hexpand(true);
            label.set_ellipsize(pango::EllipsizeMode::End);
            line.append(&label);
            row.set_child(Some(&line));
            row.update_property(&[gtk::accessible::Property::Label(section.label())]);
            imp.nav.append(&row);
            rows.push(row);
        }
        *imp.nav_rows.borrow_mut() = rows;

        // The two headings, drawn by the list itself rather than as rows of
        // their own: a heading that is a row is a heading the keyboard stops
        // on and a screen reader announces as somewhere you can go.
        imp.nav.set_header_func(|row, before| {
            let Some(section) = Section::ALL.get(row.index().max(0) as usize) else {
                return;
            };
            let previous = before
                .and_then(|earlier| Section::ALL.get(earlier.index().max(0) as usize))
                .map(|earlier| earlier.group());
            if previous == Some(section.group()) {
                row.set_header(None::<&gtk::Widget>);
                return;
            }
            let heading = kicker(section.group().label());
            heading.add_css_class("postio-settings-nav-heading");
            row.set_header(Some(&heading));
        });

        imp.nav.set_filter_func(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[upgrade_or]
            true,
            move |row| {
                Section::ALL
                    .get(row.index().max(0) as usize)
                    .is_none_or(|section| panel.matches_search(*section))
            }
        ));

        imp.nav.connect_row_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                if let Some(section) = row
                    .and_then(|row| Section::ALL.get(row.index().max(0) as usize))
                    .copied()
                    && panel.current_section() != section
                {
                    panel.show_section(section);
                }
            }
        ));

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_child(Some(&imp.nav));
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.add_css_class("postio-settings-nav");
        scroller.set_size_request(NAV_WIDTH, -1);
        // A scroll area takes the keyboard so it can be scrolled with one,
        // which means Tab stops here and a screen reader has to have
        // something to say. The rows inside are named individually; this
        // names the region they sit in.
        scroller.update_property(&[gtk::accessible::Property::Label("Settings sections")]);
        scroller
    }

    /// The pane's own heading, its one action, and the stack of eight.
    fn build_pane_area(&self) -> gtk::Box {
        let imp = self.imp();
        imp.pane_title.set_xalign(0.0);
        imp.pane_title.add_css_class("postio-settings-pane-title");
        imp.pane_description.set_xalign(0.0);
        imp.pane_description
            .add_css_class("postio-settings-pane-description");
        imp.pane_description
            .set_ellipsize(pango::EllipsizeMode::End);

        let heading = gtk::Box::new(gtk::Orientation::Vertical, 3);
        heading.set_hexpand(true);
        heading.append(&imp.pane_title);
        heading.append(&imp.pane_description);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.add_css_class("postio-settings-pane-header");
        header.append(&heading);
        imp.pane_action.set_valign(gtk::Align::Center);
        header.append(&imp.pane_action);

        // No transition at all: a pane switch is instant, the canvas's
        // motion rule for exactly this kind of move.
        imp.stack
            .set_transition_type(gtk::StackTransitionType::None);
        imp.stack.set_vexpand(true);
        for (section, pane) in [
            (Section::Accounts, &imp.accounts_pane),
            (Section::Filters, &imp.filters_pane),
            (Section::Composing, &imp.composing_pane),
            (Section::Appearance, &imp.appearance_pane),
            (Section::Keyboard, &imp.keyboard_pane),
            (Section::Sync, &imp.sync_pane),
            (Section::Privacy, &imp.privacy_pane),
            (Section::ConfigFile, &imp.config_pane),
        ] {
            pane.add_css_class("postio-settings-pane-body");
            pane.set_vexpand(true);
            imp.stack.add_named(pane, Some(section.label()));
        }

        let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        area.add_css_class("postio-settings-pane");
        area.set_hexpand(true);
        area.append(&header);
        area.append(&imp.stack);
        area
    }

    /// The strip along the foot: a state mark, what is being written, and
    /// the way out to `$EDITOR`. Identical on all eight panes.
    fn build_footer(&self) -> gtk::Box {
        let imp = self.imp();
        imp.footer_dot.add_css_class("postio-settings-footer-dot");
        imp.footer_dot.set_valign(gtk::Align::Center);
        imp.footer_target
            .add_css_class("postio-settings-footer-target");
        imp.footer_target.set_xalign(0.0);
        imp.footer_target
            .set_ellipsize(pango::EllipsizeMode::Middle);

        imp.status.set_xalign(0.0);
        imp.status.set_hexpand(true);
        imp.status.add_css_class("postio-settings-footer");
        imp.status.set_ellipsize(pango::EllipsizeMode::End);

        // The drawing puts `Open in $EDITOR` on the strip, so it is here.
        // The command already had a binding and a palette entry; what it did
        // not have was a way to find it from the settings window, which is
        // the one place a person is already thinking about the file.
        let editor = std::rc::Rc::new(crate::widgets::KeycapButton::new(
            Some(CommandId::EditConfig),
            "Open in $EDITOR",
            "postio-settings-editor",
            false,
        ));
        crate::widgets::KeycapButton::arm(&editor);
        editor.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move || panel.request_command(CommandId::EditConfig)
        ));
        let editor_widget = editor.widget();
        let _ = imp.editor_button.set(editor);

        imp.tag.add_css_class("postio-settings-tag");

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        footer.add_css_class("postio-settings-footer-row");
        footer.append(&imp.footer_dot);
        footer.append(&imp.footer_target);
        footer.append(&imp.tag);
        footer.append(&imp.status);
        footer.append(&editor_widget);
        footer
    }

    /// Redraws whichever pane is on screen, after the file changed under it.
    ///
    /// Only the visible one: the other seven redraw when they are shown, and
    /// redrawing a pane nobody is looking at on every keystroke is how a
    /// 250ms write debounce turns into a stutter.
    fn redraw_visible_pane(&self) {
        match self.imp().current.get() {
            Section::Appearance => self.redraw_ui(),
            Section::Sync => self.redraw_sync(),
            Section::Composing => self.redraw_compose(),
            Section::Keyboard => {
                self.redraw_keys();
                // The rebound count lives on the strip, so an edit to
                // `[keys]` has to reach it.
                self.refresh_footer();
            }
            _ => {}
        }
    }

    /// Drops every `[keys]` override, putting the whole keymap back to the
    /// defaults this build ships.
    fn reset_keys(&self) {
        self.apply_keys_mutation(|keys| keys.clear());
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
    fn an_interval_reads_as_the_unit_it_was_written_in() {
        assert_eq!(humanize_interval(300), "5 min");
        assert_eq!(humanize_interval(3600), "1 h");
        assert_eq!(humanize_interval(120), "2 min");
    }

    #[test]
    fn an_interval_that_is_not_a_round_minute_says_seconds_rather_than_rounding() {
        // The spin button is gone, so this line is the only thing that can
        // tell somebody their interval is 90 seconds. Rounding it to
        // "1 min" here would make the pane lie about the file.
        assert_eq!(humanize_interval(90), "90s");
        assert_eq!(humanize_interval(45), "45s");
    }

    #[test]
    fn every_section_that_owns_a_table_names_one_and_the_two_that_do_not_say_so() {
        for section in Section::ALL {
            match section {
                Section::Privacy | Section::ConfigFile => assert_eq!(
                    section.table(),
                    None,
                    "{} owns no config.toml table",
                    section.label()
                ),
                other => {
                    let table = other.table().expect("a table");
                    assert!(
                        table.starts_with('[') && table.ends_with(']'),
                        "{} names its table the way the file writes it: {table}",
                        other.label()
                    );
                    assert!(
                        table.contains(other.key()),
                        "{}'s footer label and its header key must agree: \
                         {table} vs {}",
                        other.label(),
                        other.key()
                    );
                }
            }
        }
    }

    #[test]
    fn the_sidebar_groups_are_contiguous() {
        // The list draws a heading wherever the group changes, so a section
        // filed out of order would draw MAIL twice and read as two lists.
        let groups: Vec<Group> = Section::ALL.iter().map(|s| s.group()).collect();
        let mut seen = Vec::new();
        for group in groups {
            if seen.last() != Some(&group) {
                assert!(
                    !seen.contains(&group),
                    "{group:?} appears twice in nav order, so its heading would too"
                );
                seen.push(group);
            }
        }
        assert_eq!(seen, Group::ALL.to_vec());
    }

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
        assert_eq!(find_section(SAMPLE, Section::Appearance), Some(1));
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
        assert_eq!(section_at_line(SAMPLE, 2), Some(Section::Appearance));
        assert_eq!(section_at_line(SAMPLE, 3), Some(Section::Appearance));
        assert_eq!(section_at_line(SAMPLE, 6), Some(Section::Keyboard));
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
