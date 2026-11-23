//! The settings panel (canvas 3f): `config.toml` is the settings UI, with no second store and no
//! OK/Cancel. The raw-text view writes the buffer verbatim; structured panes (Filters, Composing,
//! Sync & storage) patch only their own table with `toml_edit`'s format-preserving model and never
//! reserialize, which would reorder keys and drop comments. [`Section::Privacy`] manages the
//! remote-image allow list, which lives outside `config.toml`. [`SettingsPanel::revert`] writes the
//! last configuration that loaded cleanly back; the panel's footer raises `CommandId::EditConfig`
//! rather than spawning an editor.

use std::cell::{Cell, OnceCell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::{glib, pango};
use postio_config::compose::SignaturePlacement;
use postio_config::filters::{FilterConfig, Reorder};
use postio_config::sync::{AttachmentFetch, CheckForMail};
use postio_config::{Config, SyncConfig, patch_compose, patch_filters, patch_keys, patch_sync};
use postio_core::CommandId;
use postio_model::ids::SignatureId;
use postio_model::{Account, AccountId, MailboxRole, UnsubscribeActivation};

use crate::widgets::{
    CheckRow, ListOrEmpty, SegmentedControl, SettingsGroup, kicker, space, stat_line,
};

/// Long enough that a fast typist is not racing the disk, short enough that "applied live" reads as
/// true; the watcher's own
/// 120ms debounce runs after.
const WRITE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Written only when the mode changes to polling: see [`SettingsPanel::ensure_sync_controls`].
const POLL_EVERY_FIVE_MINUTES: u64 = 300;

/// Fixed so the pane beside it starts in the same place on every section (#1179).
pub const NAV_WIDTH: i32 = 214;

const PANE_INSET: i32 = 22;

/// One constant: the text view and its scroll region, a tab stop of its own, both announce it.
const FILE_NAME: &str = "config.toml";

const ACCOUNTS_MAX_HEIGHT: i32 = 160;

/// Not itself a `CommandId`: each needs a specific account as payload. The keyboard path resolves
/// [`SettingsPanel::focused_account`] and calls [`SettingsPanel::request_account_action`] with the
/// same variant the context menu would have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAction {
    UpdateCredential,
    Remove,
    RebuildIndex,
    SetDefault,
}

type CommandHandler = Box<dyn Fn(CommandId)>;

type AccountActionHandler = Box<dyn Fn(AccountId, AccountAction)>;

/// Never fired for the initial state [`SettingsPanel::set_accounts`] sets.
type AccountEnabledHandler = Box<dyn Fn(AccountId, bool)>;

/// An account is database state, not `config.toml` preference (ADR 0005 Q6b), so this only reports
/// what changed; the presenter has the host write it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountEdit {
    DisplayName(String),
    ImapHost(String),
    ImapPort(u16),
    SmtpHost(String),
    SmtpPort(u16),
    /// `None` is a real answer: an account can have signatures and prefer none.
    DefaultSignature(Option<SignatureId>),
    /// The path, not a `MailboxId`: it must survive the folder's row being retired and re-created
    /// when a listing loses and finds it again.
    MailboxRole(MailboxRole, Option<String>),
}

use postio_ui::settings::MAPPABLE_ROLES;

/// Three lists because they answer different questions; the resolved list is what makes "Automatic"
/// nameable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccountMailboxes {
    pub folders: Vec<String>,
    pub chosen: Vec<(MailboxRole, String)>,
    pub resolved: Vec<(MailboxRole, String)>,
    /// Distinct from "not resolved": a role can lack a folder because nobody has synced yet, which
    /// fixes itself, or because the server said no. Only the second has words worth showing, the
    /// server's own.
    pub refused: Vec<(MailboxRole, String)>,
}

type AccountEditHandler = Box<dyn Fn(AccountId, AccountEdit)>;

type TestConnectionHandler = Box<dyn Fn(AccountId)>;

type SignatureSavedHandler = Box<dyn Fn(AccountId, &SignatureDraft)>;
type SignatureDeletedHandler = Box<dyn Fn(AccountId, SignatureId)>;

/// `id` is `None` for one that does not exist yet: the store assigns it, and reporting a new
/// signature with an id would make saving an edit create a second one. No `html`: a rich editor is
/// the composer toolbar's problem (#339).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureDraft {
    pub id: Option<SignatureId>,
    pub name: String,
    pub text: String,
}

/// Three states and no fourth: a type that can only be these cannot express a spinner that silently
/// stops (#980).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Idle,
    Testing,
    /// `Err` carries what the server or its transport said.
    Answered {
        incoming: Result<(), String>,
        outgoing: Result<(), String>,
    },
}

impl ConnectionStatus {
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

    fn failed(&self) -> bool {
        matches!(
            self,
            ConnectionStatus::Answered { incoming, outgoing }
                if incoming.is_err() || outgoing.is_err()
        )
    }
}

pub use postio_ui::settings::{Group, Section, find_section, humanize_interval, section_at_line};

#[derive(Clone)]
pub struct ContactsHooks {
    pub opened: Rc<dyn Fn()>,
    pub toggle: Rc<dyn Fn(u32)>,
}

/// A free function: `Section` is `postio_ui`'s, and a GTK icon name is not for the shared crate
/// (macOS draws SF Symbols from the same enum).
pub fn icon(section: Section) -> &'static str {
    match section {
        Section::Accounts => "avatar-default-symbolic",
        Section::Contacts => "system-users-symbolic",
        Section::Filtering => "mail-mark-junk-symbolic",
        Section::Filters => "view-list-symbolic",
        Section::Composing => "document-edit-symbolic",
        Section::Appearance => "preferences-desktop-appearance-symbolic",
        Section::Keyboard => "preferences-desktop-keyboard-symbolic",
        Section::Sync => "emblem-synchronizing-symbolic",
        Section::Privacy => "security-high-symbolic",
        Section::Agents => "system-run-symbolic",
        Section::ConfigFile => "text-x-generic-symbolic",
    }
}

/// Unpinned filters are never shown in the sidebar, so this pane is the only place they are
/// visible.
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

/// The space either side of the rule is the point: flush columns read as two bordered boxes, not
/// one pane divided.
fn two_columns(left: &impl IsA<gtk::Widget>, right: &impl IsA<gtk::Widget>) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.set_widget_name(TWO_COLUMNS);
    row.add_css_class("postio-settings-columns");

    let left_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    left_box.add_css_class("postio-settings-column");
    left_box.set_hexpand(true);
    left_box.append(left);

    let right_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    right_box.add_css_class("postio-settings-column");
    right_box.set_hexpand(true);
    right_box.append(right);

    // Equal halves through a size group, not `set_homogeneous`: that shares width across every
    // child, and the third is the 1px rule, which took a third of the pane. `hexpand` alone leaves
    // the wordier column its larger natural width.
    let columns = gtk::SizeGroup::new(gtk::SizeGroupMode::Horizontal);
    columns.add_widget(&left_box);
    columns.add_widget(&right_box);

    row.append(&left_box);
    row.append(&gtk::Separator::new(gtk::Orientation::Vertical));
    row.append(&right_box);
    row
}

/// Removes rows only: a list can hold a popover too (the account rows' menu), which `remove_all`
/// would try to remove as a row forever. Stops at a row the list no longer parents.
fn clear_rows(list: &gtk::ListBox) {
    while let Some(row) = list.row_at_index(0) {
        if row.parent().as_ref() != Some(list.upcast_ref::<gtk::Widget>()) {
            break;
        }
        list.remove(&row);
    }
}

const TWO_COLUMNS: &str = "postio-settings-two-columns";

fn stack_columns(row: &gtk::Box, stacked: bool) {
    let (along, across) = if stacked {
        (gtk::Orientation::Vertical, gtk::Orientation::Horizontal)
    } else {
        (gtk::Orientation::Horizontal, gtk::Orientation::Vertical)
    };
    row.set_orientation(along);
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(rule) = widget.downcast_ref::<gtk::Separator>() {
            rule.set_orientation(across);
            rule.set_margin_top(if stacked { space::S6 } else { 0 });
            rule.set_margin_bottom(if stacked { space::S6 } else { 0 });
        }
        child = widget.next_sibling();
    }
}

fn display_path(path: &Path) -> String {
    display_path_under(path, std::env::var_os("HOME").as_deref().map(Path::new))
}

/// `home` is given because a test cannot set the environment safely.
fn display_path_under(path: &Path, home: Option<&Path>) -> String {
    home.and_then(|home| {
        path.strip_prefix(home)
            .ok()
            .map(|rest| format!("~/{}", rest.display()))
    })
    .unwrap_or_else(|| path.display().to_string())
}

// `write_atomically` is a contract with `postio_config::watch`, built to see a scratch file and a
// rename rather than an in-place write; both frontends save through it.
use postio_config::save::write_atomically;

fn row_account_id(row: &gtk::ListBoxRow) -> AccountId {
    row.widget_name()
        .strip_prefix(ACCOUNT_ROW)
        .and_then(|id| id.parse::<i64>().ok())
        .map_or(AccountId::UNASSIGNED, AccountId::new)
}

/// A widget name rather than `set_data`, which is unsafe.
const ACCOUNT_ROW: &str = "postio-account-";
pub use postio_ui::account::badge as account_badge;

fn detail_row(label: &str, control: &impl IsA<gtk::Widget>) -> gtk::Box {
    crate::widgets::field(label, control, "postio-settings-account-detail-row")
}

fn note(class: &str) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class("postio-settings-note");
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_wrap(true);
    label.set_wrap_mode(pango::WrapMode::WordChar);
    label
}

fn note_with(class: &str, text: &str) -> gtk::Label {
    let label = note(class);
    label.set_label(text);
    label
}

fn listed_box(class: &str, name: &str) -> gtk::Box {
    let column = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
    column.add_css_class("postio-settings-filtering-list");
    column.add_css_class(class);
    column.set_accessible_role(gtk::AccessibleRole::List);
    column.update_property(&[gtk::accessible::Property::Label(name)]);
    column
}

fn fill_listed(
    column: &gtk::Box,
    empty: &gtk::Label,
    listed: &[postio_ui::filtering::Listed],
    undo_class: &str,
    on_undo: &std::rc::Rc<dyn Fn(postio_ui::filtering::Undo)>,
) {
    while let Some(child) = column.first_child() {
        column.remove(&child);
    }
    for entry in listed {
        let line = gtk::Label::new(Some(&entry.says));
        line.add_css_class("postio-settings-filtering-entry");
        if !entry.acts {
            line.add_css_class("postio-settings-filtering-entry-inert");
        }
        line.set_xalign(0.0);
        line.set_hexpand(true);
        line.set_wrap(true);
        line.set_wrap_mode(pango::WrapMode::WordChar);
        line.set_selectable(false);
        let undo = crate::widgets::button::button(
            entry.undo.label(),
            crate::widgets::Kind::Ghost,
            crate::widgets::Size::Small,
        );
        undo.add_css_class("postio-settings-filtering-undo");
        undo.add_css_class(undo_class);
        undo.set_valign(gtk::Align::Center);
        let what = entry.undo.clone();
        let on_undo = std::rc::Rc::clone(on_undo);
        undo.connect_clicked(move |_| on_undo(what.clone()));
        let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
        row.set_accessible_role(gtk::AccessibleRole::ListItem);
        row.append(&line);
        row.append(&undo);
        column.append(&row);
    }
    column.set_visible(!listed.is_empty());
    empty.set_visible(listed.is_empty());
}

/// Held so the pane is updated from a fresh read, not rebuilt: repopulating a `DropDown` fires
/// `selected-notify` indistinguishable from a choice, which `SegmentedControl` and `CheckRow`
/// avoid.
pub struct SyncControls {
    pub check_for_mail: SegmentedControl,
    /// The minutes the chosen mode means, which the segmented control does not carry.
    pub interval: gtk::Label,
    pub attachments: SegmentedControl,
    pub sync_on_startup: CheckRow,
    pub notify: CheckRow,
    pub empty_junk: SegmentedControl,
    pub empty_trash: SegmentedControl,
    pub stats_size: gtk::Label,
    pub stats_accounts: gtk::Label,
    /// An `Entry`: a list a person types, not a choice between three things (ADR 0029 Q3).
    pub notify_roles: gtk::Entry,
    pub backfill_heading: gtk::Label,
    pub backfill: gtk::Box,
}

pub struct FilteringControls {
    pub switch: CheckRow,
    pub state: gtk::Label,
    pub today: gtk::Label,
    pub open: std::rc::Rc<crate::widgets::KeycapButton>,
    pub keys: crate::widgets::keyhint::KeyLine,
    pub never: gtk::Box,
    pub never_empty: gtk::Label,
    pub stopped: gtk::Box,
    pub stopped_empty: gtk::Label,
    pub after_unsubscribe: SegmentedControl,
}

pub struct ComposingControls {
    pub on_reply: SegmentedControl,
    pub on_forward: SegmentedControl,
}

type ContactTakeBack = std::rc::Rc<dyn Fn(&postio_ui::filtering::Undo)>;

type Toggled = Box<dyn Fn(bool)>;

mod imp {
    use super::*;

    pub struct SettingsPanel {
        pub header_bar: adw::HeaderBar,
        pub search: gtk::SearchEntry,
        /// Exactly one pane is ever on screen: that is the navigation model (#1179).
        pub stack: gtk::Stack,
        pub current: Cell<Section>,
        pub pane_title: gtk::Label,
        pub pane_description: gtk::Label,
        pub pane_action: gtk::Box,
        pub nav_rows: RefCell<Vec<gtk::ListBoxRow>>,
        pub footer_dot: gtk::Box,
        pub footer_target: gtk::Label,
        /// Set while `redraw_accounts` rebuilds the list, so the selection changes it causes are
        /// not mistaken for a person choosing an account.
        pub redrawing: Cell<bool>,
        pub nav_query: RefCell<String>,
        pub command: RefCell<Vec<CommandHandler>>,
        pub editor_button: OnceCell<std::rc::Rc<crate::widgets::KeycapButton>>,
        pub add_account_button: OnceCell<std::rc::Rc<crate::widgets::KeycapButton>>,
        pub narrow: Cell<bool>,
        pub folders:
            RefCell<std::collections::BTreeMap<AccountId, Vec<postio_model::mailbox::Mailbox>>>,
        #[allow(clippy::type_complexity)]
        pub backfill_handlers: RefCell<Vec<Box<dyn Fn(postio_model::ids::MailboxId, bool)>>>,
        pub accounts_pane: gtk::Box,
        pub filtering_pane: gtk::Box,
        pub filters_pane: gtk::Box,
        pub composing_pane: gtk::Box,
        pub keyboard_pane: gtk::Box,
        pub sync_pane: gtk::Box,
        pub privacy_pane: gtk::Box,
        pub contacts_pane: gtk::Box,
        pub contacts_hooks: RefCell<Option<ContactsHooks>>,
        pub agents_pane: gtk::Box,
        pub agents_list: gtk::ListBox,
        pub agents_scroller: gtk::ScrolledWindow,
        pub agents_empty: gtk::Label,
        pub agents_revoke_line: gtk::Box,
        pub agents_entry_heading: gtk::Label,
        pub agents_entry: gtk::Label,
        pub agents_summary: gtk::Label,
        pub agents_calls: gtk::Box,
        pub agents_opened: RefCell<Vec<Rc<dyn Fn()>>>,
        pub contacts_view: RefCell<Option<postio_ui::contacts_settings::SuggestionsView>>,
        pub config_pane: gtk::Box,
        /// Built on first draw and updated after, never rebuilt. Lazy (#873): constructing certain
        /// controls while the host window is still wiring its shortcut controllers corrupts
        /// keyboard routing for the window's life.
        pub sync_controls: OnceCell<SyncControls>,
        pub composing_controls: OnceCell<ComposingControls>,
        pub filtering_controls: OnceCell<FilteringControls>,
        pub filtered_today: Cell<Option<u32>>,
        pub contact_rules: RefCell<Vec<postio_config::ContactRule>>,
        pub contact_take_back: RefCell<Option<ContactTakeBack>>,
        pub keymap: RefCell<postio_core::Keymap>,
        pub tag: gtk::Label,
        pub nav: gtk::ListBox,
        pub buffer: gtk::TextBuffer,
        pub view: gtk::TextView,
        pub status: gtk::Label,
        pub revert: gtk::Button,
        pub path: RefCell<Option<PathBuf>>,
        /// Set while `load` replaces the buffer's text, so the reload is not read back as an edit
        /// and a pointless write.
        pub loading: Cell<bool>,
        pub write_source: RefCell<Option<glib::SourceId>>,
        pub dismissed: RefCell<Vec<Box<dyn Fn()>>>,
        pub last_good: RefCell<Option<String>>,
        /// Computed by `window.rs`, which has the command registry this module does not depend on.
        pub keymap_problems: RefCell<Vec<String>>,
        pub accounts_list: gtk::ListBox,
        pub egress_list: gtk::ListBox,
        pub egress_scroller: gtk::ScrolledWindow,
        pub accounts_scroller: gtk::ScrolledWindow,
        pub accounts: RefCell<Vec<Account>>,
        /// Kept apart from `accounts` because the two arrive from different places and neither may
        /// assume it runs after the other (#411).
        pub weights: RefCell<Vec<(AccountId, postio_core::event::MailFootprint)>>,
        pub attachments_included: std::cell::Cell<bool>,
        /// An id present with `None` is a real answer (a token on file with no `expires_in`); an id
        /// absent is a password account or a broker-fed one, so the row shows no validity line
        /// rather than a wrong one.
        pub token_expiries: RefCell<Vec<(AccountId, Option<std::time::SystemTime>)>>,
        pub reindex_progress: RefCell<Vec<(AccountId, (u32, u32))>>,
        /// Per account so it cannot follow the selection to another row (#1584).
        pub credential_trouble: RefCell<Vec<(AccountId, postio_ui::account::Trouble)>>,
        /// So a second right click closes the first, as `Sidebar` does with `saved_search_menu`.
        pub account_menu: RefCell<Option<gtk::PopoverMenu>>,
        pub account_action: RefCell<Vec<AccountActionHandler>>,
        pub account_enabled_changed: RefCell<Vec<AccountEnabledHandler>>,
        pub account_detail: gtk::Box,
        pub account_detail_id: RefCell<Option<AccountId>>,
        pub account_detail_display_name: OnceCell<gtk::Entry>,
        pub account_detail_imap_host: OnceCell<gtk::Entry>,
        pub account_detail_imap_port: OnceCell<gtk::Entry>,
        pub account_detail_smtp_host: OnceCell<gtk::Entry>,
        pub account_detail_smtp_port: OnceCell<gtk::Entry>,
        pub account_detail_signature: OnceCell<gtk::DropDown>,
        /// The whole row, since hiding only the control would leave a "Default signature" label
        /// with nothing beside it.
        pub account_detail_signature_row: OnceCell<gtk::Box>,
        pub account_detail_signature_ids: RefCell<Vec<SignatureId>>,
        /// Set while `open_account_detail` populates the fields, so setting an `Entry`'s text does
        /// not fire an edit.
        pub account_detail_loading: Cell<bool>,
        /// The dropdowns inside are built per open by `redraw_account_mailboxes`: they are not safe
        /// to build early.
        pub account_detail_mailboxes: gtk::Box,
        pub account_mailboxes: RefCell<Vec<(AccountId, AccountMailboxes)>>,
        pub account_edited: RefCell<Vec<AccountEditHandler>>,
        pub test_connection: RefCell<Vec<TestConnectionHandler>>,
        pub account_detail_test_button: OnceCell<gtk::Button>,
        pub signature_saved: RefCell<Vec<SignatureSavedHandler>>,
        pub signature_deleted: RefCell<Vec<SignatureDeletedHandler>>,
        pub account_detail_signature_list: OnceCell<gtk::ListBox>,
        pub account_detail_signature_ids_listed: RefCell<Vec<SignatureId>>,
        pub signature_editor: gtk::Box,
        pub signature_editor_heading: OnceCell<gtk::Label>,
        pub signature_editor_name: OnceCell<gtk::Entry>,
        pub signature_editor_text: OnceCell<gtk::TextView>,
        pub signature_editor_error: OnceCell<gtk::Label>,
        pub signature_editor_delete: OnceCell<gtk::Button>,
        pub signature_editor_on: RefCell<Option<(AccountId, Option<SignatureId>)>>,
        pub account_detail_test_status: OnceCell<gtk::Label>,
        pub filters_list: gtk::ListBox,
        pub filters_scroller: gtk::ScrolledWindow,
        pub filters_empty: gtk::Label,
        pub filters_hint_cap: gtk::Label,
        pub sync_box: gtk::Box,
        pub ui_box: gtk::Box,
        pub privacy_list: gtk::ListBox,
        pub privacy_scroller: gtk::ScrolledWindow,
        pub privacy_empty: gtk::Label,
        /// Handed in by `window.rs` rather than loaded here: a test needs a scratch path, not the
        /// real state directory.
        pub remote_image_allowlist:
            RefCell<Option<(postio_ui::allowlist::RemoteImageAllowList, PathBuf)>>,
        pub unsubscribe_list: gtk::ListBox,
        pub unsubscribe_scroller: gtk::ScrolledWindow,
        pub egress_empty: gtk::Label,
        pub unsubscribe_empty: gtk::Label,
        pub unsubscribe_activations: RefCell<Vec<UnsubscribeActivation>>,
        /// A count, not a toggle: Postio never sends a read receipt automatically, so there is only
        /// a fact to state. Always visible: zero is itself the answer.
        pub read_receipt_count: gtk::Label,
        pub gnupg_switch: gtk::Switch,
        pub gnupg_line: gtk::Label,
        pub gnupg_reload: gtk::Button,
        pub gnupg_toggled: RefCell<Option<Toggled>>,
        pub gnupg_drawing: Cell<bool>,
        pub keys_list: gtk::ListBox,
        pub keys_scroller: gtk::ScrolledWindow,
        pub capturing: RefCell<Option<CommandId>>,
        pub capture_conflict: RefCell<Option<(CommandId, String)>>,
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
                redrawing: Cell::new(false),
                nav_query: RefCell::new(String::new()),
                command: RefCell::new(Vec::new()),
                editor_button: OnceCell::new(),
                add_account_button: OnceCell::new(),
                narrow: Cell::new(false),
                folders: RefCell::default(),
                backfill_handlers: RefCell::default(),
                accounts_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                filtering_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                filters_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                composing_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                keyboard_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                sync_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                privacy_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                contacts_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                contacts_hooks: RefCell::default(),
                contacts_view: RefCell::default(),
                config_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                sync_controls: OnceCell::new(),
                composing_controls: OnceCell::new(),
                filtering_controls: OnceCell::new(),
                filtered_today: Cell::new(None),
                contact_rules: RefCell::default(),
                contact_take_back: RefCell::default(),
                keymap: RefCell::new(postio_core::Keymap::defaults().clone()),
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
                credential_trouble: RefCell::new(Vec::new()),
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
                account_detail_mailboxes: gtk::Box::new(gtk::Orientation::Vertical, 0),
                account_mailboxes: RefCell::new(Vec::new()),
                account_edited: RefCell::new(Vec::new()),
                test_connection: RefCell::new(Vec::new()),
                account_detail_test_button: OnceCell::new(),
                signature_saved: RefCell::new(Vec::new()),
                signature_deleted: RefCell::new(Vec::new()),
                account_detail_signature_list: OnceCell::new(),
                account_detail_signature_ids_listed: RefCell::new(Vec::new()),
                signature_editor: gtk::Box::new(gtk::Orientation::Vertical, 0),
                signature_editor_heading: OnceCell::new(),
                signature_editor_name: OnceCell::new(),
                signature_editor_text: OnceCell::new(),
                signature_editor_error: OnceCell::new(),
                signature_editor_delete: OnceCell::new(),
                signature_editor_on: RefCell::new(None),
                account_detail_test_status: OnceCell::new(),
                filters_list: gtk::ListBox::new(),
                filters_scroller: gtk::ScrolledWindow::new(),
                filters_empty: gtk::Label::new(Some("No saved searches yet.")),
                filters_hint_cap: crate::widgets::keyhint::cap(""),
                sync_box: gtk::Box::new(gtk::Orientation::Vertical, 0),
                ui_box: gtk::Box::new(gtk::Orientation::Vertical, 0),
                agents_pane: gtk::Box::new(gtk::Orientation::Vertical, 0),
                agents_list: gtk::ListBox::new(),
                agents_scroller: gtk::ScrolledWindow::new(),
                agents_empty: gtk::Label::new(Some(postio_ui::agent_access::NOTHING_SHARED)),
                agents_revoke_line: gtk::Box::new(gtk::Orientation::Horizontal, space::S3),
                agents_entry_heading: gtk::Label::new(None),
                agents_entry: gtk::Label::new(None),
                agents_summary: gtk::Label::new(Some(postio_ui::agent_access::NO_ACTIVITY)),
                agents_calls: gtk::Box::new(gtk::Orientation::Vertical, 0),
                agents_opened: RefCell::new(Vec::new()),
                privacy_list: gtk::ListBox::new(),
                privacy_scroller: gtk::ScrolledWindow::new(),
                privacy_empty: gtk::Label::new(Some(postio_ui::privacy::NO_ALLOWED)),
                remote_image_allowlist: RefCell::new(None),
                unsubscribe_list: gtk::ListBox::new(),
                unsubscribe_scroller: gtk::ScrolledWindow::new(),
                egress_empty: gtk::Label::new(Some(postio_ui::privacy::NO_CONNECTIONS)),
                unsubscribe_empty: gtk::Label::new(Some(postio_ui::privacy::NO_LISTS_LEFT)),
                unsubscribe_activations: RefCell::new(Vec::new()),
                read_receipt_count: gtk::Label::new(None),
                gnupg_switch: gtk::Switch::new(),
                gnupg_line: gtk::Label::new(None),
                gnupg_reload: gtk::Button::with_label("Reload GnuPG keys"),
                gnupg_toggled: RefCell::default(),
                gnupg_drawing: Cell::new(false),
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
    pub fn new() -> Self {
        Self::default()
    }

    /// A missing file opens empty: first run has nothing on disk and typing here creates it. A
    /// usable file seeds
    /// [`SettingsPanel::revert`]'s target.
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
        self.redraw_signatures();
        self.redraw_keys();
        if postio_config::validate::check_str(&text)
            .validation
            .is_valid()
        {
            self.note_known_good(&text);
        }
    }

    pub fn path(&self) -> Option<PathBuf> {
        self.imp().path.borrow().clone()
    }

    pub fn text(&self) -> String {
        let buffer = &self.imp().buffer;
        let (start, end) = buffer.bounds();
        buffer.text(&start, &end, true).to_string()
    }

    /// Test seam. Redraws explicitly: a fresh buffer is already empty, so `set_text("")` never
    /// fires `changed` and a structured pane would stay unpopulated.
    pub fn set_text(&self, text: &str) {
        self.imp().buffer.set_text(text);
        self.refresh_validity();
        self.redraw_filters();
        self.redraw_sync();
        self.redraw_signatures();
        self.redraw_keys();
        self.schedule_write();
    }

    pub fn is_valid(&self) -> bool {
        postio_config::validate::check_str(&self.text())
            .validation
            .is_valid()
    }

    pub fn connect_dismissed(&self, handler: impl Fn() + 'static) {
        self.imp().dismissed.borrow_mut().push(Box::new(handler));
    }

    fn dismiss(&self) {
        for handler in self.imp().dismissed.borrow().iter() {
            handler();
        }
    }

    /// Deliberately does not feed [`SettingsPanel::revert`]'s target: `gtk::TextBuffer::set_text`
    /// fires `changed` for the delete and the insert, so mid-edit this sees a transiently empty
    /// buffer, which is valid TOML and would overwrite the real last-good. [`note_known_good`] sees
    /// only text that reached disk.
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

    pub fn set_keymap_problems(&self, problems: &[String]) {
        *self.imp().keymap_problems.borrow_mut() = problems.to_vec();
        self.refresh_validity();
    }

    pub fn set_accounts(&self, accounts: Vec<Account>) {
        *self.imp().accounts.borrow_mut() = accounts;
        self.redraw_accounts();
        self.refresh_storage_stats();
    }

    /// `attachments_included` is `[sync] attachment_fetch == "eager"`: global, while the figures
    /// are per account, so they land on rows. Order-independent with
    /// [`set_accounts`](Self::set_accounts): whichever arrives second redraws.
    pub fn set_mail_weights(
        &self,
        weights: &[(AccountId, postio_core::event::MailFootprint)],
        attachments_included: bool,
    ) {
        let imp = self.imp();
        *imp.weights.borrow_mut() = weights.to_vec();
        imp.attachments_included.set(attachments_included);
        self.redraw_accounts();
        self.refresh_storage_stats();
    }

    /// The composition root reads the keyring and hands the result back, as this panel may not link
    /// `postio-account`.
    /// Order-independent with [`set_accounts`](Self::set_accounts).
    pub fn set_token_expiries(&self, expiries: &[(AccountId, Option<std::time::SystemTime>)]) {
        *self.imp().token_expiries.borrow_mut() = expiries.to_vec();
        self.redraw_accounts();
    }

    /// Replaces any earlier reading for the account: "where the rebuild is right now", not a log.
    pub fn set_reindex_progress(&self, account: AccountId, progress: Option<(u32, u32)>) {
        let mut readings = self.imp().reindex_progress.borrow_mut();
        readings.retain(|(id, _)| *id != account);
        if let Some(progress) = progress {
            readings.push((account, progress));
        }
        drop(readings);
        self.redraw_accounts();
    }

    pub fn set_credential_trouble(
        &self,
        account: AccountId,
        trouble: Option<postio_ui::account::Trouble>,
    ) {
        let mut found = self.imp().credential_trouble.borrow_mut();
        found.retain(|(id, _)| *id != account);
        found.extend(trouble.map(|trouble| (account, trouble)));
        drop(found);
        self.redraw_accounts();
    }

    /// `TokenStanding::Unknown` for a password account, a broker-fed one, or one not yet told
    /// about. The arithmetic and wording live in `postio_ui::account` so macOS shares them (#1584).
    fn token_standing(&self, account: AccountId) -> postio_ui::account::TokenStanding {
        let expiry = self
            .imp()
            .token_expiries
            .borrow()
            .iter()
            .find(|(id, _)| *id == account)
            .and_then(|(_, expiry)| *expiry);
        postio_ui::account::TokenStanding::of(expiry, std::time::SystemTime::now())
    }

    /// Said out loud: a rebuild makes search worse while it runs, and silence on the row would read
    /// as the button having done nothing (#981).
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

    /// `path` is where a revoke writes back to; a revoke updates the shared list too, reaching
    /// every reader of the app, open ones included.
    pub fn set_remote_image_allowlist(
        &self,
        list: postio_ui::allowlist::RemoteImageAllowList,
        path: PathBuf,
    ) {
        *self.imp().remote_image_allowlist.borrow_mut() = Some((list, path));
        self.redraw_privacy();
    }

    pub fn connect_agents_opened(&self, opened: impl Fn() + 'static) {
        self.imp().agents_opened.borrow_mut().push(Rc::new(opened));
    }

    pub fn set_agent_activity(&self, activity: postio_ui::agent_access::Activity) {
        let imp = self.imp();
        imp.agents_summary.set_label(&activity.summary);
        while let Some(child) = imp.agents_calls.first_child() {
            imp.agents_calls.remove(&child);
        }
        for call in &activity.calls {
            let line = gtk::Label::new(Some(&call.line));
            line.add_css_class("postio-settings-agents-call");
            line.set_xalign(0.0);
            line.set_hexpand(true);
            line.set_wrap(true);
            if call.refused {
                line.add_css_class("dim-label");
            }
            let when = stat_line(&call.when);
            when.set_valign(gtk::Align::Start);
            let row = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
            row.add_css_class("postio-settings-agents-call-row");
            row.set_margin_top(space::S1);
            row.set_margin_bottom(space::S1);
            row.append(&line);
            row.append(&when);
            imp.agents_calls.append(&row);
        }
    }

    pub fn set_agent_entry(&self, entry: Option<&str>) {
        let imp = self.imp();
        imp.agents_entry.set_label(entry.unwrap_or_default());
        imp.agents_entry.set_visible(entry.is_some());
        imp.agents_entry_heading.set_visible(entry.is_some());
    }

    fn build_agents_pane(&self) {
        use postio_ui::agent_access as words;
        let imp = self.imp();
        ListOrEmpty::dress(
            &imp.agents_list,
            &imp.agents_scroller,
            &imp.agents_empty,
            "postio-settings-agents-shared",
            words::SHARED_HEADING,
            Some(ACCOUNTS_MAX_HEIGHT),
        );
        imp.agents_empty
            .add_css_class("postio-settings-agents-empty");
        ListOrEmpty::show(&imp.agents_scroller, &imp.agents_empty, false);

        let note = gtk::Label::new(Some(words::REVOKE_NOTE));
        note.add_css_class("postio-settings-pane-description");
        note.add_css_class("postio-settings-agents-note");
        note.set_xalign(0.0);
        note.set_wrap(true);
        note.set_hexpand(true);
        let revoke_all = gtk::Button::with_label(words::REVOKE_ALL);
        revoke_all.add_css_class("postio-settings-agents-revoke-all");
        crate::widgets::button::style(
            &revoke_all,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
        revoke_all.set_valign(gtk::Align::Center);
        revoke_all.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.revoke_agents(None)
        ));
        imp.agents_revoke_line.append(&note);
        imp.agents_revoke_line.append(&revoke_all);

        imp.agents_entry
            .add_css_class("postio-settings-agents-entry");
        imp.agents_entry.add_css_class("monospace");
        imp.agents_entry.set_xalign(0.0);
        imp.agents_entry.set_selectable(true);
        imp.agents_entry.set_wrap(true);
        imp.agents_entry.set_wrap_mode(pango::WrapMode::Char);
        imp.agents_summary
            .add_css_class("postio-settings-agents-summary");
        imp.agents_summary.set_xalign(0.0);

        let agents = SettingsGroup::on(&imp.agents_pane);
        imp.agents_pane.add_css_class("postio-settings-agents");
        agents.section(words::SHARED_HEADING);
        agents
            .append(&imp.agents_scroller)
            .append(&imp.agents_empty)
            .note(&imp.agents_revoke_line);
        imp.agents_entry_heading.set_label(words::ENTRY_HEADING);
        imp.agents_entry_heading.add_css_class("postio-kicker");
        imp.agents_entry_heading.set_xalign(0.0);
        agents.block(&imp.agents_entry_heading);
        agents.control(&imp.agents_entry);
        self.set_agent_entry(None);
        agents.section(words::ACTIVITY_HEADING);
        agents
            .control(&imp.agents_summary)
            .append(&imp.agents_calls);
    }

    /// A file that will not parse leaves the pane as it was, insensitive, so nothing is revoked
    /// from a guess.
    fn redraw_agents(&self) {
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            imp.agents_pane.set_sensitive(false);
            return;
        };
        imp.agents_pane.set_sensitive(true);
        clear_rows(&imp.agents_list);
        let shared = postio_ui::agent_access::shared_accounts(&config.mcp);
        for account in &shared {
            imp.agents_list.append(&self.agent_row(account));
        }
        ListOrEmpty::show(&imp.agents_scroller, &imp.agents_empty, !shared.is_empty());
        imp.agents_revoke_line.set_visible(!shared.is_empty());
    }

    fn agent_row(&self, account: &postio_ui::agent_access::SharedAccount) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-agents-row");
        row.set_selectable(false);
        row.set_activatable(false);

        let address = gtk::Label::new(Some(&account.address));
        address.set_xalign(0.0);
        address.set_ellipsize(pango::EllipsizeMode::Middle);
        let facts = stat_line(&format!("{} \u{b7} {}", account.folders, account.abilities));
        let text = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
        text.set_hexpand(true);
        text.append(&address);
        text.append(&facts);

        let revoke = gtk::Button::with_label(postio_ui::agent_access::REVOKE_ONE);
        revoke.add_css_class("postio-settings-agents-revoke");
        crate::widgets::button::style(
            &revoke,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
        revoke.set_valign(gtk::Align::Center);
        revoke.update_property(&[gtk::accessible::Property::Label(&format!(
            "{} {}",
            postio_ui::agent_access::REVOKE_ONE,
            account.address
        ))]);
        let address_owned = account.address.clone();
        revoke.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.revoke_agents(Some(&address_owned))
        ));

        let box_ = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
        box_.set_margin_top(space::S2);
        box_.set_margin_bottom(space::S2);
        box_.append(&text);
        box_.append(&revoke);
        row.set_child(Some(&box_));
        row
    }

    /// Only `[mcp]` changes, through `postio_config`'s patchers over the buffer; it bites on the
    /// agent's next call, since the server reads `[mcp]` on each.
    fn revoke_agents(&self, address: Option<&str>) {
        let original = self.text();
        let patched = match address {
            Some(address) => postio_config::revoke_mcp_account(&original, address),
            None => postio_config::revoke_all_mcp(&original),
        };
        match patched {
            Ok(patched) => {
                self.imp().buffer.set_text(&patched);
                self.redraw_agents();
            }
            Err(error) => tracing::error!(%error, "could not revoke agent access: {error}"),
        }
    }

    fn redraw_privacy(&self) {
        let imp = self.imp();
        clear_rows(&imp.privacy_list);
        let senders: Vec<String> = imp
            .remote_image_allowlist
            .borrow()
            .as_ref()
            .map(|(list, _)| list.senders().map(str::to_owned).collect())
            .unwrap_or_default();
        for sender in &senders {
            imp.privacy_list.append(&self.privacy_row(sender));
        }
        ListOrEmpty::show(
            &imp.privacy_scroller,
            &imp.privacy_empty,
            !senders.is_empty(),
        );
    }

    /// Written straight back with no debounce: this is its own small file, not a
    /// keystroke-by-keystroke edit.
    fn revoke_remote_image_sender(&self, sender: &str) {
        let imp = self.imp();
        let mut guard = imp.remote_image_allowlist.borrow_mut();
        let Some((list, path)) = guard.as_mut() else {
            return;
        };
        list.revoke(sender);
        if let Err(error) = list.save_to(path) {
            tracing::error!(%error, "could not save the remote-image allow-list: {error}");
        }
        // Also the list the app's readers share, so the revoke reaches every reader, open ones
        // included.
        crate::reader::shared_allowlist(path)
            .borrow_mut()
            .revoke(sender);
        drop(guard);
        self.redraw_privacy();
    }

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

        let revoke = crate::widgets::icon_button(
            "user-trash-symbolic",
            &format!("Stop always allowing remote images from {sender}"),
        );
        revoke.add_css_class("postio-settings-privacy-revoke");
        revoke.set_tooltip_text(Some("Always ask again"));
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

    pub fn set_unsubscribe_activations(&self, activations: Vec<UnsubscribeActivation>) {
        *self.imp().unsubscribe_activations.borrow_mut() = activations;
        self.redraw_unsubscribe_activations();
    }

    pub fn connect_gnupg_keyring(&self, handler: impl Fn(bool) + 'static) {
        self.imp().gnupg_toggled.replace(Some(Box::new(handler)));
    }

    pub fn set_gnupg_keyring(&self, state: &postio_ui::settings::GnupgKeyring) {
        self.imp()
            .gnupg_line
            .set_label(&postio_ui::settings::gnupg_line(state));
    }

    #[doc(hidden)]
    pub fn gnupg_keyring_line(&self) -> String {
        self.imp().gnupg_line.label().to_string()
    }

    #[doc(hidden)]
    pub fn turn_gnupg_keyring(&self, on: bool) {
        self.imp().gnupg_switch.set_active(on);
    }

    fn gnupg_switched(&self, on: bool) {
        let imp = self.imp();
        if imp.gnupg_drawing.get() {
            return;
        }
        let original = self.text();
        match postio_config::signatures::set_gnupg_keyring(&original, on) {
            Ok(Some(patched)) => imp.buffer.set_text(&patched),
            Ok(None) => {}
            Err(error) => tracing::error!(%error, "could not patch [signatures]: {error}"),
        }
        imp.gnupg_reload.set_visible(on);
        self.ask_for_gnupg_keys(on);
    }

    fn ask_for_gnupg_keys(&self, on: bool) {
        if let Some(handler) = self.imp().gnupg_toggled.borrow().as_ref() {
            handler(on);
        }
    }

    #[doc(hidden)]
    pub fn reload_gnupg_keys(&self) {
        self.imp().gnupg_reload.emit_clicked();
    }

    fn redraw_signatures(&self) {
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        let imp = self.imp();
        imp.gnupg_drawing.set(true);
        imp.gnupg_switch.set_active(config.signatures.gnupg_keyring);
        imp.gnupg_reload
            .set_visible(config.signatures.gnupg_keyring);
        imp.gnupg_drawing.set(false);
    }

    /// A count, not a switch: Postio never sends a receipt automatically (a fixed privacy policy).
    pub fn set_read_receipt_count(&self, count: u64) {
        self.imp()
            .read_receipt_count
            .set_label(&postio_ui::privacy::read_receipts(count));
    }

    #[doc(hidden)]
    pub fn read_receipt_count_label(&self) -> String {
        self.imp().read_receipt_count.label().to_string()
    }

    fn redraw_unsubscribe_activations(&self) {
        let imp = self.imp();
        clear_rows(&imp.unsubscribe_list);
        let activations = imp.unsubscribe_activations.borrow();
        for activation in activations.iter() {
            imp.unsubscribe_list
                .append(&self.unsubscribe_activation_row(activation));
        }
        ListOrEmpty::show(
            &imp.unsubscribe_scroller,
            &imp.unsubscribe_empty,
            !activations.is_empty(),
        );
    }

    fn unsubscribe_activation_row(&self, activation: &UnsubscribeActivation) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-unsubscribe-row");
        row.set_selectable(false);

        let list = gtk::Label::new(Some(&activation.list_identifier));
        list.add_css_class("postio-settings-unsubscribe-list-identifier");
        list.set_xalign(0.0);
        list.set_hexpand(true);
        list.set_ellipsize(gtk::pango::EllipsizeMode::End);

        // Both frontends date and read an activation the same way through `postio_ui::unsubscribe`
        // (#1585).
        let when = postio_ui::unsubscribe::activated_on(activation.activated_at);
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
        row.update_property(&[gtk::accessible::Property::Label(
            &postio_ui::unsubscribe::activation_label(
                &activation.list_identifier,
                activation.activated_at,
            ),
        )]);
        row
    }

    fn redraw_accounts(&self) {
        let imp = self.imp();
        // The handler stands down for the whole rebuild: removing and re-adding rows fires
        // `row-selected`, which as gestures would close the form mid-edit, clear the account id an
        // edit needs, and reload the fields over what was typed.
        imp.redrawing.set(true);
        let open_on = *imp.account_detail_id.borrow();
        // The keyboard goes with the removed row and GTK hands it to another: the next `Return` or
        // `Delete` would act on another account (#1787). It goes back to the same account's row.
        let keyboard_on = self.focused_account();
        clear_rows(&imp.accounts_list);
        for account in imp.accounts.borrow().iter() {
            imp.accounts_list.append(&self.account_row(account));
        }
        if let Some(keyboard_on) = keyboard_on {
            let mut index = 0;
            while let Some(row) = imp.accounts_list.row_at_index(index) {
                if row_account_id(&row) == keyboard_on {
                    row.grab_focus();
                    break;
                }
                index += 1;
            }
        }
        // A redraw that dropped the selection would close a form somebody is typing in: put it back
        // on the open account first.
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
        imp.redrawing.set(false);
        // A refresh can land after this redraw found the open account gone (removed in another
        // window); an editable form over it would let an edit resurrect a deleted account.
        let open_on = *imp.account_detail_id.borrow();
        if let Some(id) = open_on
            && !imp.accounts.borrow().iter().any(|account| account.id == id)
        {
            self.close_account_detail();
            return;
        }
        // The detail view owns this section's visibility while open: a refresh must not pop the
        // list back in front (#880).
        if imp.account_detail_id.borrow().is_none() {
            imp.accounts_scroller
                .set_visible(!imp.accounts.borrow().is_empty());
        }
    }

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

    /// This list makes the privacy claim auditable. Hidden while the log is empty, which on a
    /// machine that never synced is the claim.
    pub fn set_egress(&self, entries: Vec<postio_model::egress::EgressEvent>) {
        let imp = self.imp();
        clear_rows(&imp.egress_list);
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
                    .format(postio_ui::privacy::CONNECTION_WHEN)
                    .to_string(),
            ));
            when.add_css_class("postio-settings-egress-when");
            let what = gtk::Label::new(Some(&postio_ui::privacy::connection(entry)));
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
        ListOrEmpty::show(&imp.egress_scroller, &imp.egress_empty, !entries.is_empty());
    }

    /// One name line and one mono `·`-joined metadata line, not four stacked labels.
    fn account_row(&self, account: &Account) -> gtk::ListBoxRow {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("postio-settings-account-row");
        // Selectable: selecting a row reveals the form under the list (#1179).
        row.set_selectable(true);
        row.set_widget_name(&format!("{ACCOUNT_ROW}{}", account.id.get()));

        let avatar = gtk::Label::new(Some(&postio_ui::row::initials(Some(&account.address))));
        avatar.add_css_class("postio-settings-account-avatar");
        avatar.set_valign(gtk::Align::Center);

        let address = gtk::Label::new(Some(&account.address.address));
        address.add_css_class("postio-settings-account-address");
        address.set_xalign(0.0);
        address.set_ellipsize(gtk::pango::EllipsizeMode::Middle);

        let name_line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        name_line.append(&address);
        // Words, never colour alone (ADR 0005). It says what the marker does: "primary" would
        // assert a status (#960).
        if account.is_default {
            let tag = gtk::Label::new(Some("default"));
            tag.add_css_class("postio-settings-account-default");
            tag.set_valign(gtk::Align::Center);
            tag.set_tooltip_text(Some("New messages come from this account"));
            name_line.append(&tag);
        }

        let standing = self.token_standing(account.id);
        let expired = standing.is_expired();
        let found = self
            .imp()
            .credential_trouble
            .borrow()
            .iter()
            .find(|(id, _)| *id == account.id)
            .map(|(_, trouble)| *trouble);
        let offer =
            postio_ui::account::offer(postio_ui::account::repair(account), expired, found, false);
        let mut facts = vec![account_badge(account)];
        facts.extend(self.mail_weight(account.id));
        facts.extend(standing.line());
        // An expiry already says so on its validity line; anything a test found is said here.
        facts.extend(
            offer
                .as_ref()
                .filter(|_| !expired)
                .map(|offer| offer.reason.clone()),
        );
        facts.extend(self.reindex_status(account.id));
        if !account.enabled {
            facts.push("disabled".to_owned());
        }
        let facts = facts.join(" · ");
        let metadata = stat_line(&facts);
        metadata.add_css_class("postio-settings-account-metadata");
        // The weight, sign-in and rebuild are at the line's far end, so it wraps rather than being
        // cut off.
        metadata.set_ellipsize(gtk::pango::EllipsizeMode::None);
        metadata.set_wrap(true);
        metadata.set_wrap_mode(gtk::pango::WrapMode::WordChar);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 3);
        text.set_hexpand(true);
        text.append(&name_line);
        if offer.is_some() {
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

        if let Some(label) = offer.as_ref().and_then(|offer| offer.button) {
            let reconnect = gtk::Button::with_label(label);
            reconnect.add_css_class("postio-settings-account-repair");
            crate::widgets::button::style(
                &reconnect,
                crate::widgets::button::Kind::Secondary,
                crate::widgets::button::Size::Small,
            );
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
        // The row is announced as a unit, so every line must be in the announcement or a screen
        // reader never reaches it.
        let mut announcement = format!("{}, {}", account.display_name, account.address.address);
        if account.is_default {
            announcement.push_str(", default");
        }
        announcement.push_str(&format!(", {facts}"));
        row.update_property(&[gtk::accessible::Property::Label(&announcement)]);
        row
    }

    /// `Context::Accounts` follows the keyboard into this list and no further, so the raw
    /// `config.toml` view never enters it (ADR 0005 Q6c).
    pub fn accounts_list(&self) -> gtk::ListBox {
        self.imp().accounts_list.clone()
    }

    pub fn keys_list(&self) -> gtk::ListBox {
        self.imp().keys_list.clone()
    }

    /// Focus, not selection: account rows are not selectable. `None` is a real answer: a command
    /// that guessed a row would remove an account on a keystroke aimed at nothing.
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

    /// The keyboard and mouse paths end here together: two entry points with their own handlers are
    /// two things to keep in step, and this one removes an account.
    pub fn request_account_action(&self, id: AccountId, action: AccountAction) {
        for callback in self.imp().account_action.borrow().iter() {
            callback(id, action);
        }
    }

    /// Moves the switch, not the handler, so the control seen and the column written cannot
    /// disagree.
    pub fn toggle_account_enabled(&self, id: AccountId) -> bool {
        let Some(switch) = self.enabled_switch(id) else {
            return false;
        };
        switch.set_active(!switch.is_active());
        true
    }

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

    pub fn connect_contacts(&self, opened: impl Fn() + 'static, toggle: impl Fn(u32) + 'static) {
        self.imp().contacts_hooks.replace(Some(ContactsHooks {
            opened: Rc::new(opened),
            toggle: Rc::new(toggle),
        }));
    }

    pub fn show_suggestions(&self, view: &postio_ui::contacts_settings::SuggestionsView) {
        let imp = self.imp();
        imp.contacts_view.replace(Some(view.clone()));
        let pane = &imp.contacts_pane;
        while let Some(child) = pane.first_child() {
            pane.remove(&child);
        }
        let group = SettingsGroup::on(pane);
        let intro = postio_ui::contacts_settings::intro(
            postio_ui::contacts_settings::Device::Computer,
            false,
        );
        let said = gtk::Label::new(Some(&format!("{}{}", intro.lead, intro.rest)));
        said.add_css_class("postio-settings-note");
        said.set_xalign(0.0);
        said.set_wrap(true);
        group.append(&said);
        group.section(&view.heading);
        for (index, row) in view.rows.iter().enumerate() {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
            line.add_css_class("postio-settings-switch-row");
            let words = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
            words.set_hexpand(true);
            let title = gtk::Label::new(Some(&row.title));
            title.add_css_class("postio-settings-switch-title");
            title.set_xalign(0.0);
            title.set_wrap(true);
            let detail = gtk::Label::new(Some(&row.detail));
            detail.add_css_class("dim-label");
            detail.set_xalign(0.0);
            detail.set_wrap(true);
            words.append(&title);
            words.append(&detail);
            line.append(&words);
            let switch = gtk::Switch::new();
            switch.set_active(row.on);
            switch.set_valign(gtk::Align::Center);
            switch.update_property(&[gtk::accessible::Property::Label(&row.title)]);
            let toggle = imp
                .contacts_hooks
                .borrow()
                .as_ref()
                .map(|hooks| Rc::clone(&hooks.toggle));
            let on = row.on;
            switch.connect_state_set(move |switch, wanted| {
                if wanted != on
                    && let Some(toggle) = &toggle
                {
                    toggle(index as u32);
                }
                switch.set_state(on);
                glib::Propagation::Stop
            });
            line.append(&switch);
            group.control(&line);
        }
        if let Some(error) = &view.error {
            let said = gtk::Label::new(Some(error));
            said.add_css_class("postio-settings-error");
            said.set_xalign(0.0);
            group.note(&said);
        }
        let foot = gtk::Label::new(Some(&view.footer));
        foot.add_css_class("dim-label");
        foot.set_xalign(0.0);
        foot.set_wrap(true);
        group.note(&foot);
    }

    pub fn contacts_texts(&self) -> Vec<String> {
        let mut said = Vec::new();
        let mut stack = vec![self.imp().contacts_pane.clone().upcast::<gtk::Widget>()];
        while let Some(widget) = stack.pop() {
            if let Some(label) = widget.downcast_ref::<gtk::Label>()
                && !label.text().is_empty()
            {
                said.push(label.text().to_string());
            }
            let mut child = widget.last_child();
            while let Some(previous) = child {
                child = previous.prev_sibling();
                stack.push(previous);
            }
        }
        said
    }

    pub fn suggestions_shown(&self) -> Option<postio_ui::contacts_settings::SuggestionsView> {
        self.imp().contacts_view.borrow().clone()
    }

    pub fn test_flip_suggestion(&self, index: usize) {
        let toggle = self
            .imp()
            .contacts_hooks
            .borrow()
            .as_ref()
            .map(|hooks| Rc::clone(&hooks.toggle));
        if let Some(toggle) = toggle {
            toggle(index as u32);
        }
    }

    pub fn connect_account_action(&self, handler: impl Fn(AccountId, AccountAction) + 'static) {
        self.imp()
            .account_action
            .borrow_mut()
            .push(Box::new(handler));
    }

    pub fn connect_account_enabled_changed(&self, handler: impl Fn(AccountId, bool) + 'static) {
        self.imp()
            .account_enabled_changed
            .borrow_mut()
            .push(Box::new(handler));
    }

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
        // Hidden when the account has none: a picker with nothing to choose between only says what
        // is already true (#979).
        {
            let picker = imp.account_detail_signature.get().expect("built above");
            // "None" leads: an account can have signatures and prefer none, and drawing the first
            // as chosen would say a default exists.
            let names: Vec<&str> = std::iter::once("None")
                .chain(account.signatures.iter().map(|s| s.name.as_str()))
                .collect();
            picker.set_model(Some(&gtk::StringList::new(&names)));
            *imp.account_detail_signature_ids.borrow_mut() =
                account.signatures.iter().map(|s| s.id).collect();
            let selected = account
                .default_signature_id
                .and_then(|id| account.signatures.iter().position(|s| s.id == id))
                .map_or(0, |at| at + 1);
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

        if let Some(list) = imp.account_detail_signature_list.get() {
            clear_rows(list);
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
            list.set_visible(!account.signatures.is_empty());
        }

        self.redraw_account_mailboxes(id);
        imp.account_detail.set_visible(true);
        imp.signature_editor.set_visible(false);
        *imp.signature_editor_on.borrow_mut() = None;
        // The list stays: selecting a row reveals the form under it (#1179).
        imp.accounts_scroller
            .set_visible(!imp.accounts.borrow().is_empty());
    }

    /// Built on first use, never in `build()` (#873): `SettingsPanel` is built as a hidden overlay
    /// child while `Window::new` still wires its shortcut controllers, and a widget with internal
    /// event controllers (`gtk::DropDown`, `gtk::Entry`'s `GtkText`) built then corrupted keyboard
    /// routing.
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

        // An `Entry`, not a `SpinButton`: a port is a number a person types, not steps through (ADR
        // 0029 Q3).
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
                // Committed only when it parses: a half-typed `9` on the way to `993` must not dial
                // a port.
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

        // An `Entry`, not a `SpinButton`: a port is a number a person types, not steps through (ADR
        // 0029 Q3).
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
                // Committed only when it parses: a half-typed `9` on the way to `993` must not dial
                // a port.
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

        // A dropdown over the account's own signatures (#979). Built here and hidden per account in
        // `open_account_detail`: whether it has anything to offer is a fact about the account.
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
                    .get((picker.selected() as usize).wrapping_sub(1))
                    .copied();
                panel.commit_account_edit(AccountEdit::DefaultSignature(chosen));
            }
        ));
        let signature_row = detail_row("Default signature", &signature);
        imp.account_detail.append(&signature_row);
        let _ = imp.account_detail_signature_row.set(signature_row);
        let _ = imp.account_detail_signature.set(signature);

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

        // The status line sits beside the button, not in a toast: the result is read, compared
        // against the fields above, then edited (#980).
        let test = gtk::Button::with_label("Test connection");
        test.add_css_class("postio-settings-account-detail-test");
        // Its own width: a full-bleed button reads as the screen's primary action.
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
        // A live region: the result arrives a round trip after the press.
        status.update_property(&[gtk::accessible::Property::Label("")]);
        status.set_visible(false);
        imp.account_detail.append(&status);
        let _ = imp.account_detail_test_status.set(status);

        self.build_signature_editor();
        imp.account_detail_mailboxes
            .set_orientation(gtk::Orientation::Vertical);
        imp.account_detail_mailboxes
            .add_css_class("postio-settings-account-detail-mailboxes");
        imp.account_detail.append(&imp.account_detail_mailboxes);
    }

    /// Fresh widgets on every open: the folders differ per account. The `gtk::DropDown` is built
    /// here, reached only from `open_account_detail`, per the #873 rule.
    fn redraw_account_mailboxes(&self, account: AccountId) {
        let imp = self.imp();
        let group = &imp.account_detail_mailboxes;
        while let Some(child) = group.first_child() {
            group.remove(&child);
        }

        // "Mailbox roles", the phrase the `[sync]` pane already uses, not "Folders".
        let heading = gtk::Label::new(Some("Mailbox roles"));
        heading.set_xalign(0.0);
        heading.add_css_class("postio-settings-account-detail-group");
        heading.set_margin_top(space::S6);
        heading.set_margin_bottom(space::S1);
        group.append(&heading);

        let data = imp
            .account_mailboxes
            .borrow()
            .iter()
            .find(|(id, _)| *id == account)
            .map(|(_, data)| data.clone())
            .unwrap_or_default();

        if data.folders.is_empty() {
            let empty = gtk::Label::new(Some("Folders appear after the first sync."));
            crate::widgets::empty_note(&empty, "postio-settings-account-detail-mailboxes-empty");
            group.append(&empty);
            return;
        }

        for (role, title) in MAPPABLE_ROLES {
            group.append(&detail_row(title, &self.role_dropdown(role, &data)));
        }
    }

    fn role_dropdown(&self, role: MailboxRole, data: &AccountMailboxes) -> gtk::DropDown {
        let chosen = data
            .chosen
            .iter()
            .find(|(mapped, _)| *mapped == role)
            .map(|(_, path)| path.clone());
        let resolved = data
            .resolved
            .iter()
            .find(|(mapped, _)| *mapped == role)
            .map(|(_, path)| path.as_str());
        // Named only when nothing is chosen: what automatic would say is a question only the next
        // discovery pass can answer.
        let refused = data
            .refused
            .iter()
            .find(|(mapped, _)| *mapped == role)
            .map(|(_, reason)| reason.as_str());
        let automatic = match (&chosen, resolved, refused) {
            (Some(_), _, _) => "Automatic".to_owned(),
            (None, Some(path), _) => format!("Automatic ({path})"),
            (None, None, Some(reason)) => format!("Automatic (no folder — {reason})"),
            (None, None, None) => "Automatic (no folder)".to_owned(),
        };

        let mut entries = vec![automatic];
        entries.extend(data.folders.iter().cloned());
        let dangling = chosen
            .as_ref()
            .filter(|path| !data.folders.contains(path))
            .cloned();
        if let Some(path) = &dangling {
            entries.push(format!("{path} (not on this server)"));
        }
        let labels: Vec<&str> = entries.iter().map(String::as_str).collect();
        let dropdown = gtk::DropDown::from_strings(&labels);
        dropdown.add_css_class("postio-settings-account-detail-role");
        dropdown.add_css_class(&format!(
            "postio-settings-account-detail-role-{}",
            role.as_str()
        ));
        dropdown.update_property(&[gtk::accessible::Property::Label(&format!(
            "{role:?} folder"
        ))]);
        dropdown.set_selected(match &chosen {
            None => 0,
            Some(path) => match data.folders.iter().position(|folder| folder == path) {
                Some(index) => index as u32 + 1,
                None => entries.len() as u32 - 1,
            },
        });

        // Connected after `set_selected`, so restoring what is stored never reports a change the
        // user made.
        let folders = data.folders.clone();
        dropdown.connect_selected_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |dropdown| {
                let index = dropdown.selected() as usize;
                if index == 0 {
                    panel.commit_account_edit(AccountEdit::MailboxRole(role, None));
                    return;
                }
                if let Some(path) = folders.get(index - 1) {
                    panel.commit_account_edit(AccountEdit::MailboxRole(role, Some(path.clone())));
                }
            }
        ));
        dropdown
    }

    pub fn set_account_mailboxes(&self, mailboxes: Vec<(AccountId, AccountMailboxes)>) {
        *self.imp().account_mailboxes.borrow_mut() = mailboxes;
        let open = *self.imp().account_detail_id.borrow();
        if let Some(account) = open {
            self.redraw_account_mailboxes(account);
        }
    }

    /// A second drill-in, not more rows on the detail view: a signature is a name and a body, and
    /// the detail view is one control per line.
    fn build_signature_editor(&self) {
        let imp = self.imp();
        if imp.signature_editor_name.get().is_some() {
            return;
        }

        let back = crate::widgets::icon_button("go-previous-symbolic", "Back to the account");
        back.add_css_class("postio-settings-signature-back");
        back.set_halign(gtk::Align::Start);
        back.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.close_signature_editor()
        ));
        imp.signature_editor.append(&back);

        let heading = gtk::Label::new(None);
        heading.set_xalign(0.0);
        heading.add_css_class("postio-settings-account-detail-group");
        heading.set_margin_bottom(space::S1);
        imp.signature_editor.append(&heading);
        let _ = imp.signature_editor_heading.set(heading);

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

        // Never a raw store error: "UNIQUE constraint failed" is not an answer anybody can act on.
        let error = gtk::Label::new(None);
        crate::widgets::callout(&error, "postio-settings-signature-error");
        error.set_visible(false);
        imp.signature_editor.append(&error);
        let _ = imp.signature_editor_error.set(error);

        let verbs = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        verbs.set_halign(gtk::Align::Start);
        let save = gtk::Button::with_label("Save");
        save.add_css_class("postio-settings-signature-save");
        crate::widgets::button::style(
            &save,
            crate::widgets::button::Kind::Primary,
            crate::widgets::button::Size::Regular,
        );
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

    pub fn close_account_detail(&self) {
        let imp = self.imp();
        *imp.account_detail_id.borrow_mut() = None;
        imp.account_detail.set_visible(false);
        imp.accounts_scroller
            .set_visible(!imp.accounts.borrow().is_empty());
    }

    /// The panel writes nothing (this layer may not link SQLite): the app persists it and hands
    /// back a refreshed account list or, when the store refused,
    /// [`set_signature_error`](Self::set_signature_error).
    pub fn connect_signature_saved(&self, handler: impl Fn(AccountId, &SignatureDraft) + 'static) {
        self.imp()
            .signature_saved
            .borrow_mut()
            .push(Box::new(handler));
    }

    pub fn connect_signature_deleted(&self, handler: impl Fn(AccountId, SignatureId) + 'static) {
        self.imp()
            .signature_deleted
            .borrow_mut()
            .push(Box::new(handler));
    }

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
        let address = imp
            .accounts
            .borrow()
            .iter()
            .find(|candidate| candidate.id == account)
            .map(|candidate| candidate.address.address.clone())
            .unwrap_or_default();
        imp.signature_editor_heading
            .get()
            .expect("built above")
            .set_text(&postio_ui::settings::signature_heading(
                id.is_some(),
                &address,
            ));
        let name = imp.signature_editor_name.get().expect("built above");
        let text = imp.signature_editor_text.get().expect("built above");
        name.set_text(signature.as_ref().map_or("", |s| s.name.as_str()));
        text.buffer()
            .set_text(signature.as_ref().map_or("", |s| s.text.as_str()));
        imp.signature_editor_delete
            .get()
            .expect("built above")
            .set_visible(id.is_some());
        self.set_signature_error(None);
        *imp.signature_editor_on.borrow_mut() = Some((account, id));
        imp.account_detail.set_visible(false);
        imp.signature_editor.set_visible(true);
        // The keyboard follows the eye: still in "Find a setting", the first thing typed would
        // search instead of naming the signature.
        name.grab_focus();
    }

    pub fn close_signature_editor(&self) {
        let imp = self.imp();
        *imp.signature_editor_on.borrow_mut() = None;
        imp.signature_editor.set_visible(false);
        imp.account_detail.set_visible(true);
    }

    /// The editor stays open on what was typed: a duplicate name is fixed by changing one word.
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
        // An unnamed signature cannot be shown by the picker, so refuse it here rather than in the
        // store.
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

    #[doc(hidden)]
    pub fn test_press_add_signature(&self) -> bool {
        if self.imp().account_detail_id.borrow().is_none() {
            return false;
        }
        self.open_signature_editor(None);
        true
    }

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

    #[doc(hidden)]
    pub fn test_signature_name(&self) -> String {
        self.imp()
            .signature_editor_name
            .get()
            .map(|entry| entry.text().to_string())
            .unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn test_signature_error_text(&self) -> String {
        self.imp()
            .signature_editor_error
            .get()
            .map(|label| label.text().to_string())
            .unwrap_or_default()
    }

    #[doc(hidden)]
    pub fn test_press_save_signature(&self) -> bool {
        let open = self.imp().signature_editor_on.borrow().is_some();
        if open {
            self.save_signature();
        }
        open
    }

    #[doc(hidden)]
    pub fn test_press_delete_signature(&self) -> bool {
        let deletable = matches!(*self.imp().signature_editor_on.borrow(), Some((_, Some(_))));
        if deletable {
            self.delete_signature();
        }
        deletable
    }

    /// The panel never connects to anything: it reports the gesture and the app runs
    /// `postio_session::reachability::test_connection`, answering through
    /// [`set_connection_status`](Self::set_connection_status).
    pub fn connect_test_connection(&self, handler: impl Fn(AccountId) + 'static) {
        self.imp()
            .test_connection
            .borrow_mut()
            .push(Box::new(handler));
    }

    fn ask_test_connection(&self) {
        let Some(id) = *self.imp().account_detail_id.borrow() else {
            return;
        };
        self.set_connection_status(ConnectionStatus::Testing);
        for handler in self.imp().test_connection.borrow().iter() {
            handler(id);
        }
    }

    pub fn set_connection_status(&self, status: ConnectionStatus) {
        let Some(label) = self.imp().account_detail_test_status.get() else {
            return;
        };
        let message = status.message();
        label.set_visible(!message.is_empty());
        label.set_text(&message);
        // The same string to the screen reader: a status that is only a colour cannot be read.
        label.update_property(&[gtk::accessible::Property::Label(&message)]);
        if status.failed() {
            label.add_css_class("postio-settings-account-detail-test-failed");
        } else {
            label.remove_css_class("postio-settings-account-detail-test-failed");
        }
    }

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

    /// Does nothing while `open_account_detail` is populating the fields or when the detail view is
    /// open on nothing: a stray signal during a redraw is cheaper to ignore than to chase.
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

    /// Test seam: GTK4 gives a test no way to simulate the click itself. Drive the result with
    /// `activate_action("account.<verb>", None)`.
    #[doc(hidden)]
    pub fn test_open_account_menu(&self, x: f64, y: f64) {
        self.open_account_menu(x, y);
    }

    /// A test must call this before tearing down the window, as for
    /// `Sidebar::test_close_saved_search_menu`.
    #[doc(hidden)]
    pub fn test_close_account_menu(&self) {
        if let Some(popover) = self.imp().account_menu.take() {
            popover.popdown();
        }
    }

    /// A fixed, hand-built menu: see [`AccountAction`].
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
        // On `self`, not `imp.accounts_list`, as `Sidebar` does: the popover's items resolve the
        // action by walking up from where they are clicked.
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

    /// Invalid TOML mid-edit leaves what was last drawn: a typo elsewhere is no reason to blank a
    /// pane the user is not looking at.
    fn redraw_filters(&self) {
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        clear_rows(&imp.filters_list);
        let order = filter_display_order(&config);
        let pinned = config.ordered_filter_keys();
        for key in &order {
            imp.filters_list
                .append(&self.filter_row(key, &config.filters[key], &pinned));
        }
        ListOrEmpty::show(&imp.filters_scroller, &imp.filters_empty, !order.is_empty());
    }

    /// Invalid TOML mid-edit is left alone: there is no sensible `Config` to mutate yet.
    fn apply_filters_mutation(&self, mutate: impl FnOnce(&mut Config)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config);
        match patch_filters(&original, &config.filters) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [saved_searches]: {error}"),
        }
    }

    /// Builds the controls the first time and only updates them after:
    /// [`SegmentedControl::set_selected`] and [`CheckRow::set_active`]
    /// do not report a repopulation as a person's choice (#1179).
    fn redraw_sync(&self) {
        // Controls first and unconditionally: a file that does not parse leaves the pane's values
        // unreadable, not the pane empty (#1179).
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
        for (control, days) in [
            (&controls.empty_junk, config.sync.empty_junk_after_days),
            (&controls.empty_trash, config.sync.empty_trash_after_days),
        ] {
            // A count only the file could have written selects nothing, rather than a neighbour a
            // press would overwrite.
            if let Some(index) = postio_ui::auto_empty::CHOICES
                .iter()
                .position(|choice| *choice == days)
            {
                control.set_selected(index);
            }
        }
        // Set before anything reads it back: an `Entry` has no silent setter, and `activate`, not
        // `changed`, commits it, so a redraw cannot write.
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

    /// An incomplete header pass makes every figure a lower bound
    /// ([`postio_core::event::MailFootprint::complete`]), so the line says
    /// `over 1.4 GB` rather than a number that climbs every few seconds.
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

    fn apply_sync_mutation(&self, mutate: impl FnOnce(&mut SyncConfig)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config.sync);
        match patch_sync(&original, &config.sync) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [sync]: {error}"),
        }
    }

    /// Built lazily (#873): building certain controls while the host window wires its shortcut
    /// controllers corrupts keyboard routing. The middle segment says `Every 5 min` and sets
    /// `check_for_mail = "poll"` with `poll_interval_secs = 300`; an interval that is not five
    /// minutes still selects it, and the stat line says what it is. A number is typed in the
    /// `Config file` pane.
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
                        // Only on a move to polling: pressing the segment already chosen must not
                        // overwrite an interval set on purpose.
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

        // Not a checkbox: `attachment_fetch` has three values, and a checkbox would rewrite `never`
        // to `eager` on the first touch (ADR 0029 Q1).
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

        let choices: Vec<String> = postio_ui::auto_empty::CHOICES
            .iter()
            .map(|days| postio_ui::auto_empty::label(*days))
            .collect();
        let choices: Vec<&str> = choices.iter().map(String::as_str).collect();
        let empty_junk = SegmentedControl::new("Empty Junk", &choices);
        empty_junk.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                if let Some(days) = postio_ui::auto_empty::CHOICES.get(index).copied() {
                    panel.apply_sync_mutation(move |sync| sync.empty_junk_after_days = days);
                }
            }
        ));
        let empty_trash = SegmentedControl::new("Empty Trash", &choices);
        empty_trash.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                if let Some(days) = postio_ui::auto_empty::CHOICES.get(index).copied() {
                    panel.apply_sync_mutation(move |sync| sync.empty_trash_after_days = days);
                }
            }
        ));

        let notify_roles = gtk::Entry::new();
        notify_roles.add_css_class("postio-settings-notify-roles");
        notify_roles.set_placeholder_text(Some("inbox, flagged"));
        notify_roles.update_property(&[gtk::accessible::Property::Label(
            "Mailbox roles worth a notification, comma separated",
        )]);
        // On `activate`, not `changed`: this writes the file, and committing on every keystroke
        // would put `inbo` in `config.toml` on the way to `inbox`.
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
        let left = SettingsGroup::new();
        left.section("Check for mail");
        left.control(check_for_mail.widget()).note(&interval);
        left.section("Download attachments");
        left.control(attachments.widget());

        let checks = gtk::Box::new(gtk::Orientation::Vertical, space::S2);
        checks.append(sync_on_startup.widget());
        checks.append(notify.widget());
        left.block(&checks);
        left.section("Notify for");
        notify_roles.set_halign(gtk::Align::Start);
        notify_roles.set_width_chars(24);
        left.control(&notify_roles);
        left.section("Empty Junk");
        left.control(empty_junk.widget());
        left.section("Empty Trash");
        let empty_note = stat_line(postio_ui::auto_empty::NOTE);
        empty_note.set_ellipsize(pango::EllipsizeMode::None);
        empty_note.set_wrap(true);
        left.control(empty_trash.widget()).note(&empty_note);
        let elsewhere = stat_line("remote images are allowed per sender, under Privacy");
        elsewhere.set_ellipsize(pango::EllipsizeMode::None);
        elsewhere.set_wrap(true);
        left.block(&elsewhere);

        // No `Compact index`: no command compacts an index, and a button wired to nothing is worse
        // than none. Rebuilding an index is on the account's own row.
        let stats_size = stat_line("");
        let stats_accounts = stat_line("");
        let stats = gtk::Box::new(gtk::Orientation::Vertical, 6);
        stats.add_css_class("postio-stat-block");
        stats.append(&stats_size);
        stats.append(&stats_accounts);

        let sync_now = gtk::Button::with_label("Sync now");
        crate::widgets::button::style(
            &sync_now,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
        sync_now.set_halign(gtk::Align::Start);
        sync_now.set_margin_top(6);
        sync_now.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.request_command(CommandId::Refresh)
        ));
        stats.append(&sync_now);

        let right = SettingsGroup::new();
        right.section("Local store");
        right.control(&stats);

        let backfill_heading = right.section("Back up locally");
        let backfill = gtk::Box::new(gtk::Orientation::Vertical, space::S1);
        backfill.add_css_class("postio-settings-backfill");
        right.control(&backfill);
        backfill_heading.set_visible(false);
        backfill.set_visible(false);

        let columns = two_columns(left.widget(), right.widget());
        stack_columns(&columns, imp.narrow.get());
        imp.sync_pane.append(&columns);

        let _ = imp.sync_controls.set(SyncControls {
            check_for_mail,
            interval,
            attachments,
            sync_on_startup,
            notify,
            empty_junk,
            empty_trash,
            stats_size,
            stats_accounts,
            notify_roles,
            backfill_heading,
            backfill,
        });
        self.redraw_backfill();
        imp.sync_controls.get().expect("just set")
    }

    fn redraw_keys(&self) {
        self.ensure_capture_controller();
        let imp = self.imp();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        clear_rows(&imp.keys_list);
        for spec in postio_core::registry::all()
            .filter(|spec| spec.requires.offered_by(postio_core::Frontend::Focus))
        {
            imp.keys_list.append(&self.key_row(spec, &config.keys));
        }
    }

    /// Built on first `redraw_keys`, not `build()`: #873/#880 found widgets with their own event
    /// controllers, built while `Window::new` wires its shortcuts, corrupting keyboard routing.
    /// Deferred on the same precaution.
    fn ensure_capture_controller(&self) {
        let imp = self.imp();
        if imp.capture_controller_installed.get() {
            return;
        }
        imp.capture_controller_installed.set(true);

        // Capture phase: it must see a keypress before the row's own rebind `Button`, or the
        // Space/Enter pressing that button would be swallowed by its activation. Stops propagation
        // only while capturing.
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
                // Held one main-loop turn: this fires mid-propagation, and resolving synchronously
                // makes `redraw_keys` tear down the widget
                // GTK is still routing the event to (as `MessageList::hold`).
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
            Err(error) => tracing::error!(%error, "could not patch [keys]: {error}"),
        }
    }

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
        // The keycap is the control (Design/screens/22), and while waiting it says `press a key…`
        // in place of the key about to be lost.
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

    /// `Escape` always cancels rather than becoming the binding, so "get me out" is distinct from
    /// "bind Escape here".
    fn resolve_capture(&self, keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) {
        let Some(command) = *self.imp().capturing.borrow() else {
            return;
        };
        if keyval == gtk::gdk::Key::Escape {
            *self.imp().capturing.borrow_mut() = None;
            self.redraw_keys();
            return;
        }
        let Some(chord) = crate::keys::chord(keyval, state) else {
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

    /// Test seam: a window's own key handling resolves a synthetic keypress directly and never
    /// reaches this panel's capture controller, so this exercises the logic it calls.
    #[doc(hidden)]
    pub fn test_capture_key(&self, keyval: gtk::gdk::Key, state: gtk::gdk::ModifierType) {
        self.resolve_capture(keyval, state);
    }

    /// The host's count; `None` while unknown, and the line says nothing rather than a guess.
    pub fn set_filtered_today(&self, count: Option<u32>) {
        if self.imp().filtered_today.replace(count) != count
            && self.imp().filtering_controls.get().is_some()
        {
            self.redraw_filtering();
        }
    }

    pub fn set_contact_rules(&self, rules: Vec<postio_config::ContactRule>) {
        if self.imp().contact_rules.replace(rules.clone()) != rules
            && self.imp().filtering_controls.get().is_some()
        {
            self.redraw_filtering();
        }
    }

    /// A person's pin or marker is theirs, not the file's to edit.
    pub fn connect_contact_take_back(
        &self,
        take_back: impl Fn(&postio_ui::filtering::Undo) + 'static,
    ) {
        *self.imp().contact_take_back.borrow_mut() = Some(std::rc::Rc::new(take_back));
    }

    /// Controls first and unconditionally, as on Sync & storage: a file that does not parse leaves
    /// them showing what they last showed.
    fn redraw_filtering(&self) {
        let controls = self.ensure_filtering();
        let Ok(config) = Config::from_toml_str(&self.text()) else {
            return;
        };
        let imp = self.imp();
        let focus = config.focus.with_contacts(&imp.contact_rules.borrow());
        let page =
            postio_ui::filtering::page(&focus, imp.filtered_today.get(), &imp.keymap.borrow());
        controls.switch.set_active(page.on);
        controls.after_unsubscribe.set_selected(
            postio_ui::unsubscribe::SETTING_CHOICES
                .iter()
                .position(|choice| *choice == config.focus.after_unsubscribe)
                .unwrap_or(0),
        );
        controls.state.set_label(&page.state);
        controls
            .today
            .set_label(page.today.as_deref().unwrap_or_default());
        controls.today.set_visible(page.today.is_some());
        controls
            .open
            .set_key(imp.keymap.borrow().binding(CommandId::GoToFiltered));
        controls.keys.set(&page.keys);
        controls.keys.widget().set_visible(!page.keys.is_empty());
        let panel = self.downgrade();
        let on_undo: std::rc::Rc<dyn Fn(postio_ui::filtering::Undo)> =
            std::rc::Rc::new(move |undo| {
                if let Some(panel) = panel.upgrade() {
                    panel.take_back(&undo);
                }
            });
        fill_listed(
            &controls.never,
            &controls.never_empty,
            &page.never,
            "postio-settings-filtering-undo-never",
            &on_undo,
        );
        controls.never_empty.set_label(&page.never_empty);
        fill_listed(
            &controls.stopped,
            &controls.stopped_empty,
            &page.stopped,
            "postio-settings-filtering-undo-stopped",
            &on_undo,
        );
    }

    fn ensure_filtering(&self) -> &FilteringControls {
        use postio_ui::filtering;
        let imp = self.imp();
        if let Some(controls) = imp.filtering_controls.get() {
            return controls;
        }

        let switch = CheckRow::new(filtering::SWITCH);
        switch
            .widget()
            .add_css_class("postio-settings-filtering-switch");
        switch.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |on| panel.apply_filtering(on)
        ));
        let state = note("postio-settings-filtering-state");

        let today = gtk::Label::new(None);
        today.add_css_class("postio-settings-filtering-today");
        today.set_xalign(0.0);
        let open = std::rc::Rc::new(crate::widgets::KeycapButton::new(
            Some(CommandId::GoToFiltered),
            filtering::OPEN,
            "postio-settings-filtering-open",
            false,
        ));
        crate::widgets::KeycapButton::arm(&open);
        open.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move || panel.request_command(CommandId::GoToFiltered)
        ));
        let count_row = gtk::Box::new(gtk::Orientation::Horizontal, space::S3);
        count_row.add_css_class("postio-settings-filtering-count");
        count_row.append(&today);
        count_row.append(&open.widget());
        let keys = crate::widgets::keyhint::KeyLine::new("postio-settings-filtering-keys");

        let never = listed_box("postio-settings-filtering-never", filtering::NEVER);
        let never_empty = note("postio-settings-filtering-never-empty");
        let stopped = listed_box("postio-settings-filtering-stopped", filtering::STOPPED);
        let stopped_empty = note("postio-settings-filtering-stopped-empty");
        stopped_empty.set_label(filtering::STOPPED_EMPTY);

        let choices: Vec<&str> = postio_ui::unsubscribe::SETTING_CHOICES
            .iter()
            .map(|choice| postio_ui::unsubscribe::sweep_verb(*choice))
            .collect();
        let after_unsubscribe = SegmentedControl::new("After you unsubscribe", &choices);
        after_unsubscribe.connect_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |index| {
                if let Some(action) = postio_ui::unsubscribe::SETTING_CHOICES.get(index) {
                    panel.apply_after_unsubscribe(*action);
                }
            }
        ));
        let after_unsubscribe_note = note("postio-settings-after-unsubscribe-note");
        after_unsubscribe_note.set_label(postio_ui::unsubscribe::SETTING_NOTE);
        after_unsubscribe_note.set_ellipsize(pango::EllipsizeMode::None);
        after_unsubscribe_note.set_wrap(true);

        let group = SettingsGroup::on(&imp.filtering_pane);
        group.control(switch.widget()).note(&state);
        group.section(filtering::FILTERED);
        group.control(&count_row);
        group.note(&note_with(
            "postio-settings-filtering-kept",
            filtering::KEPT,
        ));
        group.note(keys.widget());
        group.section(filtering::NEVER);
        group.control(&note_with(
            "postio-settings-filtering-guards",
            filtering::GUARDS,
        ));
        group.note(&never).note(&never_empty);
        group.section(filtering::STOPPED);
        group.control(&stopped).control(&stopped_empty);
        group.section(postio_ui::unsubscribe::SETTING_HEADING);
        group
            .control(after_unsubscribe.widget())
            .note(&after_unsubscribe_note);

        let _ = imp.filtering_controls.set(FilteringControls {
            switch,
            state,
            today,
            open,
            keys,
            never,
            never_empty,
            stopped,
            stopped_empty,
            after_unsubscribe,
        });
        imp.filtering_controls.get().expect("just set")
    }

    fn take_back(&self, undo: &postio_ui::filtering::Undo) {
        use postio_ui::filtering::Undo;
        let text = self.text();
        if let Ok(config) = Config::from_toml_str(&text) {
            let focus = config
                .focus
                .with_contacts(&self.imp().contact_rules.borrow());
            let theirs = match undo {
                Undo::Never(entry) => focus.contact_pinned(entry).is_some(),
                Undo::Marker { sender, kind } => focus.contact_stopped(sender, kind).is_some(),
            };
            if theirs {
                let take_back = self.imp().contact_take_back.borrow().clone();
                if let Some(take_back) = take_back {
                    take_back(undo);
                }
                return;
            }
        }
        let written = match undo {
            Undo::Never(entry) => postio_config::focus_edit::set_never(&text, entry, false),
            Undo::Marker { sender, kind } => {
                postio_config::focus_edit::set_stop_marker(&text, sender, kind, false)
            }
        };
        match written {
            Ok(Some(written)) => self.imp().buffer.set_text(&written),
            Ok(None) => {}
            Err(error) => tracing::error!(%error, "could not take back a filter entry: {error}"),
        }
    }

    fn apply_after_unsubscribe(&self, action: postio_config::AfterUnsubscribe) {
        match postio_config::focus_edit::set_after_unsubscribe(&self.text(), action) {
            Ok(Some(written)) => self.imp().buffer.set_text(&written),
            Ok(None) => {}
            Err(error) => {
                tracing::error!(%error, "could not write [focus] after_unsubscribe: {error}");
            }
        }
    }

    fn apply_filtering(&self, on: bool) {
        match postio_config::focus_edit::set_filtering(&self.text(), on) {
            Ok(Some(written)) => self.imp().buffer.set_text(&written),
            Ok(None) => {}
            Err(error) => tracing::error!(%error, "could not write [focus] filtering: {error}"),
        }
    }

    fn redraw_compose(&self) {
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

        let group = SettingsGroup::new();
        group.section("Signature on a reply");
        group.control(on_reply.widget());
        group.section("Signature on a forward");
        group.control(on_forward.widget());
        group.block(&stat_line(
            "a reply answers a fragment · a forward hands the whole message on",
        ));
        let column = group.widget();
        column.set_margin_start(PANE_INSET);
        column.set_margin_end(PANE_INSET);
        column.set_margin_top(PANE_INSET);
        imp.composing_pane.append(column);

        let _ = imp.composing_controls.set(ComposingControls {
            on_reply,
            on_forward,
        });
        imp.composing_controls.get().expect("just set")
    }

    fn apply_compose_mutation(&self, mutate: impl FnOnce(&mut postio_config::ComposeConfig)) {
        let original = self.text();
        let Ok(mut config) = Config::from_toml_str(&original) else {
            return;
        };
        mutate(&mut config.compose);
        match patch_compose(&original, &config.compose) {
            Ok(patched) => self.imp().buffer.set_text(&patched),
            Err(error) => tracing::error!(%error, "could not patch [compose]: {error}"),
        }
    }

    /// `pinned_keys` is the sidebar's own order, so up/down can tell a row from the first or last
    /// pinned filter, which is not its position in the combined list.
    fn filter_row(
        &self,
        key: &str,
        filter: &FilterConfig,
        pinned_keys: &[String],
    ) -> gtk::ListBoxRow {
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

        // A checkbox, not a switch: pinned is a value written to the file, and switches are for
        // acts (ADR 0029 Q2).
        let pinned = CheckRow::new("Pinned");
        pinned.set_active(filter.pinned);
        pinned
            .widget()
            .update_property(&[gtk::accessible::Property::Label(&format!(
                "Pin {title} across the command bar"
            ))]);
        pinned.connect_toggled(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            #[strong]
            key,
            move |active| {
                let key = key.clone();
                panel.apply_filters_mutation(move |config| {
                    config.set_filter_pinned(&key, active);
                });
            }
        ));

        let position = pinned_keys.iter().position(|candidate| *candidate == key);
        let up = crate::widgets::icon_button("go-up-symbolic", &format!("Move {title} up"));
        up.add_css_class("postio-settings-filter-up");
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

        let down = crate::widgets::icon_button("go-down-symbolic", &format!("Move {title} down"));
        down.add_css_class("postio-settings-filter-down");
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

        let delete = crate::widgets::icon_button(
            "user-trash-symbolic",
            &format!("Delete the saved search {title}"),
        );
        delete.add_css_class("postio-settings-filter-delete");
        delete.set_tooltip_text(Some("Delete"));
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
        pinned.widget().set_valign(gtk::Align::Center);
        box_.append(pinned.widget());
        const RUN: [CommandId; 4] = [
            CommandId::SavedSearch1,
            CommandId::SavedSearch2,
            CommandId::SavedSearch3,
            CommandId::SavedSearch4,
        ];
        if filter.pinned
            && let Some(command) = position.and_then(|index| RUN.get(index))
            && let Some(key) = postio_ui::hints::key(&self.imp().keymap.borrow(), *command)
        {
            let cap = crate::widgets::keyhint::cap(&key);
            cap.add_css_class("postio-settings-filter-key");
            cap.set_valign(gtk::Align::Center);
            box_.append(&cap);
        }
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

    /// Separate from the buffer-driven validity check, so a `$EDITOR` save updates what "last good"
    /// means without clobbering an edit in progress here.
    pub fn note_known_good(&self, text: &str) {
        *self.imp().last_good.borrow_mut() = Some(text.to_string());
    }

    /// Does nothing when there is no path or nothing has validated: pretending there is something
    /// to revert to could overwrite a first, never-saved edit with nothing.
    pub fn revert(&self) {
        let imp = self.imp();
        let Some(text) = imp.last_good.borrow().clone() else {
            return;
        };
        let Some(path) = imp.path.borrow().clone() else {
            return;
        };
        if let Err(error) = write_atomically(&path, &text) {
            tracing::error!(path = %path.display(), %error, "cannot revert the config file: {error}");
            return;
        }
        imp.loading.set(true);
        imp.buffer.set_text(&text);
        imp.loading.set(false);
        self.refresh_validity();
        imp.status
            .set_label("Reverted to the last configuration that loaded without error.");
    }

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
            tracing::error!(path = %path.display(), %error, "cannot save the config file: {error}");
        }
    }

    /// Built here because the search filters this sidebar, and a control whose behaviour lives in
    /// another module is how three keycap implementations happened.
    pub fn header_bar(&self) -> adw::HeaderBar {
        self.imp().header_bar.clone()
    }

    pub fn search_field(&self) -> gtk::SearchEntry {
        let imp = self.imp();
        if imp.search.parent().is_some() {
            imp.header_bar.remove(&imp.search);
        }
        imp.search.clone()
    }

    pub fn is_capturing(&self) -> bool {
        self.imp().capturing.borrow().is_some()
    }

    pub fn current_section(&self) -> Section {
        self.imp().current.get()
    }

    pub fn show_section(&self, section: Section) {
        let imp = self.imp();
        let previous = imp.current.replace(section);
        imp.stack.set_visible_child_name(section.label());
        imp.pane_title.set_label(section.label());
        imp.pane_description.set_label(section.description());
        // The box stays in the header on every pane so the title does not shift sideways as you
        // move down the sidebar.
        imp.pane_action.set_visible(section == Section::Accounts);

        // Panes that build controls on first draw (#873) draw here, the first moment one is looked
        // at.
        match section {
            Section::Contacts => {
                let opened = imp
                    .contacts_hooks
                    .borrow()
                    .as_ref()
                    .map(|hooks| Rc::clone(&hooks.opened));
                if let Some(opened) = opened {
                    opened();
                }
            }
            Section::Filtering => self.redraw_filtering(),
            Section::Sync => self.redraw_sync(),
            Section::Composing => self.redraw_compose(),
            Section::Agents => {
                self.redraw_agents();
                let opened: Vec<Rc<dyn Fn()>> = imp.agents_opened.borrow().clone();
                for opened in opened {
                    opened();
                }
            }
            Section::Keyboard => {
                self.ensure_capture_controller();
                self.redraw_keys();
            }
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

    /// Matches the section's name and the words its pane is about, not the controls: a person
    /// typing "idle" wants to be taken to Sync & storage.
    fn apply_search(&self, query: &str) {
        *self.imp().nav_query.borrow_mut() = query.trim().to_lowercase();
        self.imp().nav.invalidate_filter();
    }

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

    pub fn set_narrow(&self, narrow: bool) {
        let imp = self.imp();
        if imp.narrow.replace(narrow) == narrow {
            return;
        }
        imp.stack.set_hhomogeneous(!narrow);
        {
            let mut child = imp.sync_pane.first_child();
            while let Some(widget) = child {
                if let Some(row) = widget
                    .downcast_ref::<gtk::Box>()
                    .filter(|row| row.widget_name() == TWO_COLUMNS)
                {
                    stack_columns(row, narrow);
                }
                child = widget.next_sibling();
            }
        }
    }

    pub fn listed_sections(&self) -> Vec<Section> {
        Section::ALL
            .into_iter()
            .filter(|section| self.shown(*section) && self.matches_search(*section))
            .collect()
    }

    pub fn shown(&self, section: Section) -> bool {
        section.shown_in(postio_core::Frontend::Focus)
    }

    /// Never for the state [`SettingsPanel::set_account_folders`] draws.
    pub fn connect_backfill_exclusion_changed(
        &self,
        handler: impl Fn(postio_model::ids::MailboxId, bool) + 'static,
    ) {
        self.imp()
            .backfill_handlers
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// Empty does nothing: there is no account to name.
    pub fn set_account_folders(&self, folders: Vec<postio_model::mailbox::Mailbox>) {
        let Some(account) = folders.first().map(|folder| folder.account_id) else {
            return;
        };
        self.imp().folders.borrow_mut().insert(account, folders);
        if self.imp().sync_controls.get().is_some() {
            self.redraw_backfill();
        }
    }

    pub fn reveal_backfill(&self) {
        let Some(controls) = self.imp().sync_controls.get() else {
            return;
        };
        if !controls.backfill_heading.is_visible() {
            return;
        }
        if let Some(scroller) = controls
            .backfill_heading
            .ancestor(gtk::ScrolledWindow::static_type())
            .and_downcast::<gtk::ScrolledWindow>()
            && let Some(bounds) = controls
                .backfill_heading
                .compute_bounds(&scroller.child().unwrap())
        {
            let adjustment = scroller.vadjustment();
            adjustment.set_value(f64::from(bounds.y()) - 24.0);
        }
    }

    /// `None` when it draws no check for that folder.
    pub fn backfill_check(&self, path: &str) -> Option<gtk::CheckButton> {
        let controls = self.imp().sync_controls.get()?;
        let mut child = controls.backfill.first_child();
        while let Some(widget) = child {
            if let Some(check) = widget.downcast_ref::<gtk::CheckButton>()
                && check.widget_name() == path
            {
                return Some(check.clone());
            }
            child = widget.next_sibling();
        }
        None
    }

    fn redraw_backfill(&self) {
        let imp = self.imp();
        let Some(controls) = imp.sync_controls.get() else {
            return;
        };
        while let Some(child) = controls.backfill.first_child() {
            controls.backfill.remove(&child);
        }
        let folders = imp.folders.borrow();
        let accounts = imp.accounts.borrow();
        let several = folders.len() > 1;
        for (account, mailboxes) in folders.iter() {
            if several {
                let address = accounts
                    .iter()
                    .find(|row| row.id == *account)
                    .map(|row| row.address.address.clone())
                    .unwrap_or_default();
                let heading = stat_line(&address);
                heading.add_css_class("postio-settings-backfill-account");
                controls.backfill.append(&heading);
            }
            for mailbox in mailboxes.iter().filter(|mailbox| mailbox.selectable) {
                let name = postio_ui::sidebar::path_label(mailbox, mailboxes);
                let check = CheckRow::new(&name);
                check.set_active(!mailbox.backfill_excluded);
                check.widget().set_widget_name(&mailbox.path);
                check
                    .widget()
                    .update_property(&[gtk::accessible::Property::Label(&format!(
                        "Back up {name} locally",
                    ))]);
                let id = mailbox.id;
                check.connect_toggled(glib::clone!(
                    #[weak(rename_to = panel)]
                    self,
                    move |active| {
                        for handler in panel.imp().backfill_handlers.borrow().iter() {
                            handler(id, !active);
                        }
                    }
                ));
                controls.backfill.append(check.widget());
            }
        }
        let any = !folders.is_empty();
        controls.backfill_heading.set_visible(any);
        controls.backfill.set_visible(any);
    }

    /// The panel raises `CommandId`s and runs none: the window already has the one dispatch every
    /// keystroke and menu goes through, and a second path from a button to the runtime is how
    /// surfaces disagree about what `Refresh` means.
    pub fn connect_command(&self, handler: impl Fn(CommandId) + 'static) {
        self.imp().command.borrow_mut().push(Box::new(handler));
    }

    fn request_command(&self, command: CommandId) {
        for handler in self.imp().command.borrow().iter() {
            handler(command);
        }
    }

    /// The key comes from the resolved keymap, so a `[keys]` rebind changes what the button says
    /// when it changes the keyboard.
    pub fn set_keymap(&self, keymap: &postio_core::Keymap) {
        if let Some(editor) = self.imp().editor_button.get() {
            editor.set_key(keymap.binding(CommandId::EditConfig));
        }
        if let Some(add) = self.imp().add_account_button.get() {
            add.set_key(keymap.binding(CommandId::AddAccount));
        }
        let save = keymap.binding(CommandId::SaveSearch);
        self.imp().filters_hint_cap.set_label(save.unwrap_or(""));
        if let Some(line) = self.imp().filters_hint_cap.parent() {
            line.set_visible(save.is_some());
        }
        self.imp().keymap.replace(keymap.clone());
        if self.imp().filtering_controls.get().is_some() {
            self.redraw_filtering();
        }
    }

    fn refresh_footer(&self) {
        let imp = self.imp();
        let section = imp.current.get();
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

    pub fn footer_target_text(&self) -> String {
        self.imp().footer_target.label().to_string()
    }

    fn cursor_line(&self) -> usize {
        let buffer = &self.imp().buffer;
        let iter = buffer.iter_at_mark(&buffer.get_insert());
        usize::try_from(iter.line()).unwrap_or(0)
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-settings");

        self.set_accessible_role(gtk::AccessibleRole::Dialog);
        self.update_property(&[gtk::accessible::Property::Label("Settings")]);

        self.build_header_bar();

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

        imp.accounts_list.set_activate_on_single_click(true);
        imp.accounts_list.connect_row_selected(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_, row| {
                if panel.imp().redrawing.get() {
                    return;
                }
                match row {
                    Some(row) => {
                        let id = row_account_id(row);
                        if id.is_assigned() {
                            panel.open_account_detail(id);
                        }
                    }
                    None => panel.close_account_detail(),
                }
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

        imp.account_detail
            .add_css_class("postio-settings-account-detail");
        imp.account_detail.set_visible(false);
        imp.signature_editor
            .add_css_class("postio-settings-signature-editor");
        imp.signature_editor.set_visible(false);
        // Not built here: constructing these this early would repeat #873 (see
        // `ensure_account_detail_fields`).

        let add_account = std::rc::Rc::new(crate::widgets::KeycapButton::new(
            Some(CommandId::AddAccount),
            "Add account",
            "postio-settings-add-account",
            true,
        ));
        crate::widgets::KeycapButton::arm(&add_account);
        add_account.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move || panel.request_command(CommandId::AddAccount)
        ));
        imp.pane_action.append(&add_account.widget());
        let _ = imp.add_account_button.set(add_account);

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

        ListOrEmpty::dress(
            &imp.filters_list,
            &imp.filters_scroller,
            &imp.filters_empty,
            "postio-settings-filters",
            "Saved searches",
            None,
        );
        imp.filters_scroller.set_visible(false);
        imp.filters_empty.set_visible(false);

        imp.filters_pane.append(&imp.filters_scroller);
        imp.filters_pane.append(&imp.filters_empty);
        let hint = gtk::Box::new(gtk::Orientation::Horizontal, space::S2);
        hint.add_css_class("postio-settings-filters-hint");
        hint.set_halign(gtk::Align::Start);
        let hint_words = gtk::Label::new(Some("Save a search from the command bar with"));
        hint_words.add_css_class("postio-settings-pane-description");
        hint.append(&hint_words);
        hint.append(&imp.filters_hint_cap);
        imp.filters_pane.append(&hint);

        ListOrEmpty::dress(
            &imp.privacy_list,
            &imp.privacy_scroller,
            &imp.privacy_empty,
            "postio-settings-privacy",
            "Senders always allowed to load remote images",
            Some(ACCOUNTS_MAX_HEIGHT),
        );
        // Empty from the start: a heading with nothing under it is wrong both before the list is
        // handed over and after.
        ListOrEmpty::show(&imp.privacy_scroller, &imp.privacy_empty, false);

        ListOrEmpty::dress(
            &imp.unsubscribe_list,
            &imp.unsubscribe_scroller,
            &imp.unsubscribe_empty,
            "postio-settings-unsubscribe",
            "Mailing lists left through one-click unsubscribe",
            Some(ACCOUNTS_MAX_HEIGHT),
        );
        imp.unsubscribe_scroller.set_visible(false);
        imp.unsubscribe_empty.set_visible(false);

        imp.read_receipt_count
            .add_css_class("postio-settings-read-receipt-count");
        imp.read_receipt_count.set_xalign(0.0);
        imp.read_receipt_count.set_wrap(true);
        self.set_read_receipt_count(0);

        ListOrEmpty::dress(
            &imp.egress_list,
            &imp.egress_scroller,
            &imp.egress_empty,
            "postio-settings-egress",
            "Recent connections",
            Some(ACCOUNTS_MAX_HEIGHT),
        );
        imp.egress_scroller.set_visible(false);

        let privacy = SettingsGroup::on(&imp.privacy_pane);
        privacy.section(postio_ui::privacy::ALLOWED);
        privacy
            .append(&imp.privacy_scroller)
            .append(&imp.privacy_empty);
        privacy.section(postio_ui::privacy::LISTS_LEFT);
        privacy
            .append(&imp.unsubscribe_scroller)
            .append(&imp.unsubscribe_empty);
        privacy.section(postio_ui::privacy::READ_RECEIPTS);
        privacy.append(&imp.read_receipt_count);
        privacy.section("Signatures");
        let gnupg_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let gnupg_label = gtk::Label::new(Some("Use keys from my GnuPG keyring"));
        gnupg_label.set_xalign(0.0);
        gnupg_label.set_hexpand(true);
        gnupg_row.append(&gnupg_label);
        imp.gnupg_switch.set_valign(gtk::Align::Center);
        imp.gnupg_switch
            .update_property(&[gtk::accessible::Property::Label(
                "Use keys from my GnuPG keyring",
            )]);
        gnupg_row.append(&imp.gnupg_switch);
        imp.gnupg_line.set_xalign(0.0);
        imp.gnupg_line.set_wrap(true);
        imp.gnupg_line.add_css_class("dim-label");
        imp.gnupg_line.set_label(&postio_ui::settings::gnupg_line(
            &postio_ui::settings::GnupgKeyring::Off,
        ));
        imp.gnupg_switch.connect_active_notify(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |switch| panel.gnupg_switched(switch.is_active())
        ));
        imp.gnupg_reload.set_halign(gtk::Align::Start);
        imp.gnupg_reload.set_visible(false);
        imp.gnupg_reload.connect_clicked(glib::clone!(
            #[weak(rename_to = panel)]
            self,
            move |_| panel.ask_for_gnupg_keys(true)
        ));
        privacy
            .append(&gnupg_row)
            .append(&imp.gnupg_line)
            .append(&imp.gnupg_reload);
        privacy.section(postio_ui::privacy::CONNECTIONS);
        privacy
            .append(&imp.egress_scroller)
            .append(&imp.egress_empty);

        self.build_agents_pane();

        imp.keys_list.add_css_class("postio-settings-keys-list");
        imp.keys_list.set_selection_mode(gtk::SelectionMode::None);
        imp.keys_list
            .update_property(&[gtk::accessible::Property::Label("Keybindings")]);

        imp.keys_scroller.set_child(Some(&imp.keys_list));
        imp.keys_scroller
            .set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        imp.keys_scroller.set_vexpand(true);
        imp.keys_scroller.add_css_class("postio-settings-keys");
        imp.keys_scroller
            .update_property(&[gtk::accessible::Property::Label("Keybindings")]);

        // No `Import mutt bindings` or key-set switcher: this build has one set of defaults and no
        // importer.
        let reset_keys = gtk::Button::with_label("Reset to defaults");
        crate::widgets::button::style(
            &reset_keys,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
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
        view_scroller.update_property(&[gtk::accessible::Property::Label(FILE_NAME)]);

        imp.revert.add_css_class("postio-settings-revert");
        crate::widgets::button::style(
            &imp.revert,
            crate::widgets::button::Kind::Secondary,
            crate::widgets::button::Size::Small,
        );
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
        // Not `redraw_sync()` here: `Window::new` builds this panel as a hidden child while wiring
        // its shortcut controllers, and building certain controls then corrupted keyboard routing
        // (#873). Panes populate from `show_section`, which cannot run before the window is up.
        self.show_section(Section::Accounts);
    }

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
            let icon = gtk::Image::from_icon_name(icon(section));
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

        // Headings drawn by the list itself, not as rows: a heading row is a keyboard stop and
        // announced as somewhere you can go.
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
                    .is_none_or(|section| panel.shown(*section) && panel.matches_search(*section))
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
        scroller.update_property(&[gtk::accessible::Property::Label("Settings sections")]);
        scroller
    }

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

        imp.stack
            .set_transition_type(gtk::StackTransitionType::None);
        imp.stack.set_vexpand(true);
        for (section, pane) in [
            (Section::Accounts, &imp.accounts_pane),
            (Section::Contacts, &imp.contacts_pane),
            (Section::Filtering, &imp.filtering_pane),
            (Section::Filters, &imp.filters_pane),
            (Section::Composing, &imp.composing_pane),
            (Section::Keyboard, &imp.keyboard_pane),
            (Section::Sync, &imp.sync_pane),
            (Section::Privacy, &imp.privacy_pane),
            (Section::Agents, &imp.agents_pane),
            (Section::ConfigFile, &imp.config_pane),
        ] {
            pane.add_css_class("postio-settings-pane-body");
            pane.set_vexpand(true);
            // A pane with no list of its own scrolls whole when the window is shorter: Focus's
            // dialog is the window's height less 80.
            if matches!(
                section,
                Section::Filtering
                    | Section::Composing
                    | Section::Sync
                    | Section::Privacy
                    | Section::Agents
            ) {
                let scroller = gtk::ScrolledWindow::builder()
                    .hscrollbar_policy(gtk::PolicyType::Never)
                    .vscrollbar_policy(gtk::PolicyType::Automatic)
                    .propagate_natural_height(true)
                    .child(pane)
                    .build();
                imp.stack.add_named(&scroller, Some(section.label()));
                continue;
            }
            imp.stack.add_named(pane, Some(section.label()));
        }

        let area = gtk::Box::new(gtk::Orientation::Vertical, 0);
        area.add_css_class("postio-settings-pane");
        area.set_hexpand(true);
        area.append(&header);
        area.append(&imp.stack);
        area
    }

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
        imp.status.set_wrap(true);
        imp.status.set_wrap_mode(pango::WrapMode::WordChar);

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

        imp.tag.add_css_class("postio-chip-base");
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

    /// Only the visible pane: the others redraw when shown, and redrawing every keystroke turns the
    /// 250ms debounce into a stutter.
    fn redraw_visible_pane(&self) {
        match self.imp().current.get() {
            Section::Filtering => self.redraw_filtering(),
            Section::Sync => self.redraw_sync(),
            Section::Composing => self.redraw_compose(),
            Section::Agents => self.redraw_agents(),
            Section::Keyboard => {
                self.redraw_keys();
                self.refresh_footer();
            }
            _ => {}
        }
    }

    fn reset_keys(&self) {
        self.apply_keys_mutation(|keys| keys.clear());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_display_order_puts_pinned_filters_first_in_sidebar_order_then_unpinned_alphabetically()
     {
        let config = Config::from_toml_str(
            "\
[saved_searches.b]
query = \"subject:b\"
pinned = false

[saved_searches.zebra]
query = \"is:unread\"
pinned = true
order = 1

[saved_searches.apple]
query = \"has:attach\"
pinned = true
order = 0

[saved_searches.a]
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

    #[test]
    fn a_path_under_home_is_shown_with_a_tilde() {
        assert_eq!(
            display_path_under(
                Path::new("/home/example/.config/postio/config.toml"),
                Some(Path::new("/home/example"))
            ),
            "~/.config/postio/config.toml"
        );
    }

    #[test]
    fn a_path_outside_home_is_shown_verbatim() {
        assert_eq!(
            display_path_under(
                Path::new("/etc/postio/config.toml"),
                Some(Path::new("/home/example"))
            ),
            "/etc/postio/config.toml"
        );
    }
}
