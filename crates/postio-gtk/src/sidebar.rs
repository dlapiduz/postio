//! The sidebar: the account, its folders with counts, and the sync status
//! line pinned to the bottom.
//!
//! Canvas 1b, top to bottom: the account address as a kicker, the special-use
//! folders in a fixed order with their counts set in IBM Plex Mono, the
//! ordinary folders under a second kicker, then `idle · imap` and
//! `last sync 12s` at the foot.
//!
//! # Widgets here, not a custom `snapshot()`
//!
//! The message list draws its rows by hand, because it has tens of thousands
//! of them and 40px at scroll speed leaves no room for per-row boxes. The
//! sidebar has a dozen rows that never scroll under the finger, and it needs
//! what `GtkListBox` already does properly: arrow-key navigation, selection,
//! and rows a screen reader announces. Drawing it by hand would trade all of
//! that for microseconds nobody can measure.
//!
//! # What is not wired yet
//!
//! [`Sidebar::set_mailboxes`] and [`Sidebar::set_status`] are the whole input
//! surface, and they take the domain's own types. The folder data arrives when
//! the mailbox repository lands (E2.3) and the status when the sync engine
//! starts emitting it (E5.9); until then the sidebar renders honestly — no
//! folders, offline, never synced.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::ConnectionState;
use postio_model::ids::MailboxId;
use postio_model::mailbox::{Mailbox, MailboxRole};

/// What to call when the user picks a folder.
type SelectionHandler = Box<dyn Fn(MailboxId)>;

/// What to call when the user picks a saved search.
type SearchSelectionHandler = Box<dyn Fn(String)>;

/// What to call when messages are dropped on a folder.
type DropHandler = Box<dyn Fn(crate::list_view::Dragged, MailboxId)>;

/// A pinned `[filters]` entry, as the sidebar needs it: a name to show and a
/// query to hand back when the row is picked.
///
/// Not `postio_config::FilterConfig` itself, whose name is a map key rather
/// than a field: the widget takes a flat list of rows to draw, the same
/// shape `set_mailboxes` already takes, so the caller carries the config
/// schema's own key-value split and this stays a plain display value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedSearch {
    /// The `[filters.<key>]` key -- the stable identity a rename, reorder
    /// or delete acts on (#292). Never shown; [`SavedSearch::name`] is what
    /// draws.
    pub key: String,
    /// What the row shows: the display name the user chose, or `key` if
    /// they never renamed it.
    pub name: String,
    /// The query text this row hands back when activated.
    pub query: String,
}

/// What a saved search's context menu asked for, and which one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SavedSearchAction {
    /// Change the display name.
    Rename,
    /// Move one place earlier in the list.
    MoveUp,
    /// Move one place later in the list.
    MoveDown,
    /// Remove it.
    Delete,
}

/// What to call when a saved search's context menu picks an action.
type SavedSearchActionHandler = Box<dyn Fn(String, SavedSearchAction)>;

/// What to call when a folder's context menu flips whether it participates
/// in background backfill (ADR 0016, #350) — the mailbox, and what it
/// should become.
type BackfillExclusionHandler = Box<dyn Fn(MailboxId, bool)>;

/// The protocol the status line names. v1 is IMAP only (CLAUDE.md).
const PROTOCOL: &str = "imap";

/// What the status line has to say.
///
/// Assembled from `ConnectionChanged` and `SyncProgress` on the core event
/// stream. `last_sync` is an [`Instant`] rather than a wall-clock time because
/// the line shows an age, and an age must not jump when the system clock is
/// corrected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStatus {
    /// Where the account stands with its server.
    pub state: ConnectionState,
    /// When the last sync completed.
    pub last_sync: Option<Instant>,
    /// Completed and expected units of a long resync.
    pub progress: Option<(u32, u32)>,
    /// Settled and queued message *bodies*, while a backfill is running.
    ///
    /// Kept apart from [`progress`](Self::progress) rather than folded into
    /// it, because the two phases mean different things to someone looking
    /// at the sidebar: a list still arriving cannot be read, and bodies
    /// still arriving can. See issue #74.
    pub backfill: Option<(u32, u32)>,
    /// Why the connection is failing, phrased for the user.
    pub detail: Option<String>,
}

impl Default for SyncStatus {
    /// What Postio shows before it has ever reached a server.
    fn default() -> Self {
        SyncStatus {
            state: ConnectionState::Offline,
            last_sync: None,
            progress: None,
            backfill: None,
            detail: None,
        }
    }
}

impl SyncStatus {
    /// The two lines the canvas draws, as of `now`.
    pub fn lines(&self, now: Instant) -> (String, String) {
        (
            format!("{} · {PROTOCOL}", self.state_word()),
            self.detail_line(now),
        )
    }

    fn state_word(&self) -> String {
        match self.state {
            ConnectionState::Offline => "offline".to_string(),
            ConnectionState::Connecting => "connecting".to_string(),
            ConnectionState::Failing { .. } => "error".to_string(),
            ConnectionState::Online if self.syncing().is_some() => "syncing".to_string(),
            // The list is complete and the bodies are not. Its own word,
            // because "syncing" already means the list and "idle" was the
            // lie issue #74 was filed about. It matches what the reading
            // pane says about a message it has no body for, which is the
            // same fact seen from the other end.
            ConnectionState::Online if self.filling().is_some() => "downloading".to_string(),
            ConnectionState::Online => "idle".to_string(),
        }
    }

    /// How many messages the pass that is running has fetched, if one is.
    ///
    /// One question, asked once, and both lines answer from it — which is the
    /// whole of `postio-qhz.6`. The first live sync said "0% synced" and
    /// "never synced" together because the two lines were reading different
    /// sources: progress from `SyncProgress`, "never synced" from a
    /// `last_synced_at` that only moves when a pass *completes*. Neither was
    /// wrong on its own terms and the pair was useless.
    ///
    /// `progress` is `Some` exactly while a pass is in flight — `SyncTracker`
    /// clears it on any connection change and when `done` reaches `total` —
    /// so its presence is the answer to "is anything happening".
    fn syncing(&self) -> Option<u32> {
        match self.progress {
            // A pass with nothing to reach never started.
            Some((_, 0)) => None,
            Some((done, total)) if done < total => Some(done),
            _ => None,
        }
    }

    /// How many bodies the backfill has settled, if one is running.
    ///
    /// `None` once the queue has drained, so a finished backfill falls back
    /// to the ordinary idle line rather than sticking at `2000 of 2000` —
    /// the same trap `syncing` fell into and the same answer.
    fn filling(&self) -> Option<(u32, u32)> {
        match self.backfill {
            // A queue with nothing in it is not a backfill in progress.
            Some((_, 0)) => None,
            Some((done, total)) if done < total => Some((done, total)),
            _ => None,
        }
    }

    /// The second line: the reason it is failing, or how long ago it worked.
    ///
    /// The reason wins. "last sync 4h" is not what someone needs to read when
    /// the password has expired.
    fn detail_line(&self, now: Instant) -> String {
        if matches!(self.state, ConnectionState::Failing { .. })
            && let Some(detail) = &self.detail
        {
            return detail.clone();
        }
        // A pass that is running says what it has, not when it last finished
        // and not a percentage. The denominator is `UIDNEXT - 1` — the highest
        // UID the pass *could* reach, which expunged messages leave gaps in —
        // so a pass routinely finishes well short of it and a percentage of it
        // is a number that does not mean what it looks like. A count that
        // climbs answers "is anything happening", which is the only question
        // this line is being asked during a first sync.
        //
        // No thousands separator: the folder counts beside it are written
        // `4291`, and two number formats in one column read as two kinds of
        // number.
        if let Some(fetched) = self.syncing() {
            return format!("fetched {fetched}");
        }
        // Unlike the list pass, a backfill knows its real denominator: every
        // message that has entered the queue is in exactly one of the counts
        // `BackfillProgress` keeps. So this one can honestly say "of", which
        // "fetched 1204" above deliberately cannot.
        if let Some((done, total)) = self.filling() {
            return format!("bodies {done} of {total}");
        }
        match self.last_sync {
            Some(at) => format!("last sync {}", age(now.saturating_duration_since(at))),
            None => "never synced".to_string(),
        }
    }

    /// How long until the age on the second line would read differently.
    ///
    /// `None` when nothing is ticking. The point is to not wake the process up
    /// once a second forever: seconds only matter while the answer is in
    /// seconds.
    pub fn refresh_interval(&self, now: Instant) -> Option<Duration> {
        let elapsed = now.saturating_duration_since(self.last_sync?);
        Some(match elapsed.as_secs() {
            ..60 => Duration::from_secs(1),
            60..3600 => Duration::from_secs(30),
            _ => Duration::from_secs(300),
        })
    }
}

/// A duration in the canvas' compact form: `12s`, `4m`, `3h`, `2d`.
pub(crate) fn age(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    match seconds {
        0..60 => format!("{seconds}s"),
        60..3600 => format!("{}m", seconds / 60),
        3600..86_400 => format!("{}h", seconds / 3600),
        _ => format!("{}d", seconds / 86_400),
    }
}

/// The count a folder shows, or `None` when it shows none.
///
/// Straight off the canvas: Inbox 12 unread, Flagged 3 flagged, Drafts 2 in
/// total, and nothing at all beside Sent or Archive. A count of zero is not
/// drawn — an empty column is quieter than a row of noughts.
pub fn count_for(mailbox: &Mailbox) -> Option<u32> {
    let counts = &mailbox.counts;
    let count = match mailbox.role {
        // A draft you have not finished is not "unread".
        MailboxRole::Drafts => counts.total,
        MailboxRole::Flagged => counts.flagged,
        // Nothing arrives in these unread, so a count would only ever be
        // "how much have you kept", which is not a thing to nag about.
        MailboxRole::Sent | MailboxRole::Archive | MailboxRole::Trash | MailboxRole::Junk => 0,
        MailboxRole::Inbox | MailboxRole::Regular => counts.unread,
    };
    (count > 0).then_some(count)
}

/// Where a role sits in the sidebar, or `None` for an ordinary folder.
///
/// The canvas' order — Inbox, Flagged, Drafts, Sent, Archive — with the two
/// folders it does not happen to draw after them.
fn role_order(role: MailboxRole) -> Option<u8> {
    match role {
        MailboxRole::Inbox => Some(0),
        MailboxRole::Flagged => Some(1),
        MailboxRole::Drafts => Some(2),
        MailboxRole::Sent => Some(3),
        MailboxRole::Archive => Some(4),
        MailboxRole::Junk => Some(5),
        MailboxRole::Trash => Some(6),
        MailboxRole::Regular => None,
    }
}

/// Split the mailboxes into the two sections the canvas draws, each in order.
///
/// Unselectable folders — `\Noselect` containers that exist only to hold a
/// hierarchy — are dropped: a row you cannot open is a row that wastes a
/// keystroke.
pub fn sections(mailboxes: &[Mailbox]) -> (Vec<Mailbox>, Vec<Mailbox>) {
    let mut special: Vec<Mailbox> = Vec::new();
    let mut ordinary: Vec<Mailbox> = Vec::new();

    for mailbox in mailboxes.iter().filter(|m| m.selectable) {
        // One row per role (#501): an account that has passed through more
        // than one client holds two folders per role, and a special section
        // that renamed both to the role drew `Sent, Sent, Archive, Archive`.
        // Only the primary — the mailbox actions route to — gets the role
        // treatment; its twin is an ordinary folder under its server name.
        match role_order(mailbox.role) {
            Some(_) if primary_within(mailbox, mailboxes) => special.push(mailbox.clone()),
            _ => ordinary.push(mailbox.clone()),
        }
    }

    special.sort_by_key(|m| (role_order(m.role).unwrap_or(u8::MAX), m.name.clone()));
    ordinary.sort_by_key(|m| m.path.to_lowercase());
    (special, ordinary)
}

/// One row of the ordinary folder tree (#324), positioned in the hierarchy
/// the server reported: the mailbox itself, how deep it nests, and whether
/// it has children to disclose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRow {
    pub mailbox: Mailbox,
    /// Ancestors between this row and a root, capped at [`MAX_DEPTH`].
    pub depth: u8,
    /// Whether this row has at least one child in the tree, whatever its
    /// current expansion state.
    pub has_children: bool,
}

/// Nesting deeper than this renders at the same indent as this depth: the
/// sidebar column has finite width, and pushing a name out of it to indent
/// correctly is worse than an indent that stops being literal.
const MAX_DEPTH: u8 = 4;

/// Flatten the ordinary folders into the order the sidebar draws them:
/// depth-first, each level sorted the way the flat list has always been
/// sorted, a folder's children immediately beneath it and hidden while it is
/// collapsed.
///
/// `collapsed` names the folders currently closed; everything else with
/// children is open, which is why a fresh account — nothing collapsed yet —
/// renders exactly as flat-but-correctly-indented as it would before this
/// existed, rather than defaulting to a wall of closed rows.
///
/// A `\Noselect` container (`Mailbox::selectable == false`) still gets a row
/// when it has children, so the hierarchy it organizes can be opened even
/// though it cannot be opened as a mailbox — see #324's acceptance. A
/// `\Noselect` folder with nothing under it gets no row at all: nothing to
/// open and nothing to toggle is a row that wastes a keystroke, same as
/// today's flat list already decided in [`sections`].
///
/// A child whose parent was never listed by the server (`parent_id` points
/// at nothing in `mailboxes`, or is `None`) renders as its own root — exactly
/// what `postio-sync::discover::link_parents` already promises: "the folder
/// is still perfectly usable; it just sits at the top."
pub fn folder_rows(mailboxes: &[Mailbox], collapsed: &HashSet<MailboxId>) -> Vec<FolderRow> {
    let ordinary: Vec<&Mailbox> = mailboxes
        .iter()
        .filter(|m| role_order(m.role).is_none() || !primary_within(m, mailboxes))
        .collect();
    let present: HashSet<MailboxId> = ordinary.iter().map(|m| m.id).collect();

    let mut children: HashMap<MailboxId, Vec<&Mailbox>> = HashMap::new();
    for m in &ordinary {
        if let Some(parent) = m.parent_id
            && present.contains(&parent)
        {
            children.entry(parent).or_default().push(m);
        }
    }
    for list in children.values_mut() {
        list.sort_by_key(|m| m.path.to_lowercase());
    }

    let mut roots: Vec<&Mailbox> = ordinary
        .iter()
        .copied()
        .filter(|m| !m.parent_id.is_some_and(|p| present.contains(&p)))
        .collect();
    roots.sort_by_key(|m| m.path.to_lowercase());

    let mut out = Vec::new();
    for root in roots {
        walk_folder_tree(root, 0, &children, collapsed, &mut out);
    }
    out
}

fn walk_folder_tree<'a>(
    mailbox: &'a Mailbox,
    depth: u8,
    children: &HashMap<MailboxId, Vec<&'a Mailbox>>,
    collapsed: &HashSet<MailboxId>,
    out: &mut Vec<FolderRow>,
) {
    let kids = children.get(&mailbox.id);
    let has_children = kids.is_some_and(|k| !k.is_empty());
    if !mailbox.selectable && !has_children {
        return;
    }
    out.push(FolderRow {
        mailbox: mailbox.clone(),
        depth: depth.min(MAX_DEPTH),
        has_children,
    });
    if has_children && !collapsed.contains(&mailbox.id) {
        for child in kids.into_iter().flatten() {
            walk_folder_tree(child, depth + 1, children, collapsed, out);
        }
    }
}

/// Every ancestor of `id`, nearest first, so the caller can open all of them.
///
/// A folder selected while an ancestor is collapsed must still be reachable —
/// see [`Sidebar::select`] — and this is what tells it which parents to open.
pub fn ancestors_of(mailboxes: &[Mailbox], id: MailboxId) -> Vec<MailboxId> {
    let by_id: HashMap<MailboxId, &Mailbox> = mailboxes.iter().map(|m| (m.id, m)).collect();
    let mut out = Vec::new();
    let mut current = by_id.get(&id).and_then(|m| m.parent_id);
    while let Some(parent) = current {
        if !by_id.contains_key(&parent) {
            break;
        }
        out.push(parent);
        current = by_id.get(&parent).and_then(|m| m.parent_id);
    }
    out
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Sidebar {
        pub account: gtk::Label,
        pub special: gtk::ListBox,
        pub ordinary: gtk::ListBox,
        pub ordinary_section: gtk::Box,
        pub saved: gtk::ListBox,
        pub saved_section: gtk::Box,
        pub status_state: gtk::Label,
        pub status_detail: gtk::Label,
        pub status: RefCell<SyncStatus>,
        pub tick: RefCell<Option<glib::SourceId>>,
        pub selected: RefCell<Vec<SelectionHandler>>,
        pub search_selected: RefCell<Vec<SearchSelectionHandler>>,
        pub saved_search_action: RefCell<Vec<SavedSearchActionHandler>>,
        /// The saved-search context menu currently up, if any -- so a
        /// second right-click before the first closes replaces it rather
        /// than stacking a second grabbing popup on top.
        pub saved_search_menu: RefCell<Option<gtk::PopoverMenu>>,
        pub backfill_exclusion_changed: RefCell<Vec<BackfillExclusionHandler>>,
        /// The ordinary-folder context menu currently up, if any -- the same
        /// shape as `saved_search_menu`, and for the same reason.
        pub folder_menu: RefCell<Option<gtk::PopoverMenu>>,
        pub dropped: RefCell<Vec<DropHandler>>,
        /// Set while a selection is being applied programmatically, so
        /// restoring one does not look like the user clicking it.
        pub echoing: std::cell::Cell<bool>,
        /// The full mailbox list [`Sidebar::set_mailboxes`] was last given —
        /// kept so a toggle can re-flatten the tree, and a select can find a
        /// row's ancestors, without the caller handing the list back.
        pub mailboxes: RefCell<Vec<Mailbox>>,
        /// Folders the ordinary tree is showing collapsed (#324).
        pub collapsed: RefCell<HashSet<MailboxId>>,
        pub collapsed_changed: RefCell<Vec<Box<dyn Fn()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "PostioSidebar";
        type Type = super::Sidebar;
        type ParentType = adw::Bin;
    }

    impl ObjectImpl for Sidebar {
        fn constructed(&self) {
            self.parent_constructed();
            self.obj().build();
        }

        fn dispose(&self) {
            if let Some(tick) = self.tick.borrow_mut().take() {
                tick.remove();
            }
        }
    }

    impl WidgetImpl for Sidebar {}
    impl BinImpl for Sidebar {}
}

glib::wrapper! {
    /// The folder list and the sync status line.
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends adw::Bin, gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl Default for Sidebar {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl Sidebar {
    /// An empty sidebar: no account, no folders, offline.
    pub fn new() -> Self {
        Self::default()
    }

    fn build(&self) {
        let imp = self.imp();
        self.add_css_class("postio-sidebar");
        self.set_hexpand(false);

        imp.account.add_css_class("postio-kicker");
        imp.account.set_xalign(0.0);
        imp.account.set_ellipsize(pango::EllipsizeMode::Middle);

        // The folders scroll; the account line above them and the sync status
        // below them do not. An account with fifteen folders — which is an
        // ordinary account, not a large one — asked for 949px of a 700px
        // window, and GTK answered by clipping: four folders were unreachable
        // with no scrollbar to say so, and the status line was pushed off the
        // bottom entirely. `postio-qhz.4`.
        //
        // Pinning the status rather than letting it scroll away with the
        // folders is the point of splitting them: `idle · imap / last sync
        // 12s` is the answer to "is anything happening", and an answer you
        // have to scroll for is one you will not look at.
        let folders = gtk::Box::new(gtk::Orientation::Vertical, 0);
        folders.append(&folder_list(&imp.special));

        let heading = gtk::Label::new(Some("Folders"));
        heading.add_css_class("postio-kicker");
        heading.set_xalign(0.0);

        let rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        rule.add_css_class("postio-rule");

        imp.ordinary_section
            .set_orientation(gtk::Orientation::Vertical);
        imp.ordinary_section.append(&rule);
        imp.ordinary_section.append(&heading);
        imp.ordinary_section.append(&folder_list(&imp.ordinary));
        // Nothing to list until there are folders that are not special-use.
        imp.ordinary_section.set_visible(false);
        folders.append(&imp.ordinary_section);

        let saved_rule = gtk::Separator::new(gtk::Orientation::Horizontal);
        saved_rule.add_css_class("postio-rule");
        let saved_heading = gtk::Label::new(Some("Saved searches"));
        saved_heading.add_css_class("postio-kicker");
        saved_heading.set_xalign(0.0);

        imp.saved_section
            .set_orientation(gtk::Orientation::Vertical);
        imp.saved_section
            .add_css_class("postio-saved-searches-section");
        imp.saved_section.append(&saved_rule);
        imp.saved_section.append(&saved_heading);
        imp.saved.add_css_class("postio-saved-searches");
        imp.saved_section.append(&search_list(&imp.saved));
        // Nothing to list until a search has been pinned.
        imp.saved_section.set_visible(false);
        folders.append(&imp.saved_section);

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        // Not a tab stop: the lists inside already move with the keyboard, so
        // stopping here would be a stop that does nothing and says nothing.
        scroller.set_focusable(false);
        scroller.set_child(Some(&folders));
        // A folder below the fold cannot be dropped on otherwise: the pointer
        // is held down, so the wheel is awkward and the scrollbar is
        // elsewhere. Without this the only way to reach it is to abandon the
        // drag, scroll, and start again.
        crate::autoscroll::attach(&scroller);

        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&imp.account);
        column.append(&scroller);

        for label in [&imp.status_state, &imp.status_detail] {
            label.add_css_class("postio-status");
            label.set_xalign(0.0);
            label.set_ellipsize(pango::EllipsizeMode::End);
        }
        let status = gtk::Box::new(gtk::Orientation::Vertical, 0);
        status.add_css_class("postio-status-line");
        status.append(&imp.status_state);
        status.append(&imp.status_detail);
        // One landmark, read as a unit, rather than two stray lines.
        status.set_accessible_role(gtk::AccessibleRole::Status);
        column.append(&status);

        // Selecting in one list clears the other two: together they are one
        // list of folders, plus the saved searches, drawn in blocks.
        let special = imp.special.clone();
        let ordinary = imp.ordinary.clone();
        let saved = imp.saved.clone();

        imp.special.connect_row_selected(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            ordinary,
            #[weak]
            saved,
            move |_, row| {
                let Some(row) = row else { return };
                ordinary.unselect_all();
                saved.unselect_all();
                if sidebar.imp().echoing.get() {
                    return;
                }
                let id = MailboxId::new(row_id(row));
                for callback in sidebar.imp().selected.borrow().iter() {
                    callback(id);
                }
            }
        ));
        imp.ordinary.connect_row_selected(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            special,
            #[weak]
            saved,
            move |_, row| {
                let Some(row) = row else { return };
                special.unselect_all();
                saved.unselect_all();
                if sidebar.imp().echoing.get() {
                    return;
                }
                let id = MailboxId::new(row_id(row));
                // A `\Noselect` container has nothing to open — clicking it
                // toggles its children instead, since that is the only
                // thing selecting it could mean (#324).
                if !sidebar.is_openable(id) {
                    sidebar.toggle(id);
                    return;
                }
                for callback in sidebar.imp().selected.borrow().iter() {
                    callback(id);
                }
            }
        ));
        imp.saved.connect_row_selected(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            special,
            #[weak]
            ordinary,
            move |_, row| {
                let Some(row) = row else { return };
                special.unselect_all();
                ordinary.unselect_all();
                if sidebar.imp().echoing.get() {
                    return;
                }
                let query = row_query(row);
                for callback in sidebar.imp().search_selected.borrow().iter() {
                    callback(query.clone());
                }
            }
        ));

        // Rename, reorder, delete (#292): a saved search has no keyboard
        // path into it yet -- see the doc on `connect_saved_search_action`
        // for why this stays mouse-only for now -- so this is the one way
        // in, the same right-click-a-row idiom `list_view.rs`'s message
        // context menu already uses.
        let menu = gtk::GestureClick::new();
        menu.set_button(gtk::gdk::BUTTON_SECONDARY);
        menu.set_propagation_phase(gtk::PropagationPhase::Capture);
        menu.connect_pressed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |_, _, x, y| {
                sidebar.open_saved_search_menu(x, y);
            }
        ));
        imp.saved.add_controller(menu);

        // Skip/resume background backfill (ADR 0016, #350): a folder's own
        // context menu, the same right-click-a-row idiom as the
        // saved-search one just above. On both sections, not only the
        // ordinary tree — ADR 0016's own motivating example, Junk, is a
        // special-use folder and lives in `special`, not `ordinary`.
        for list in [&imp.special, &imp.ordinary] {
            let folder_menu = gtk::GestureClick::new();
            folder_menu.set_button(gtk::gdk::BUTTON_SECONDARY);
            folder_menu.set_propagation_phase(gtk::PropagationPhase::Capture);
            folder_menu.connect_pressed(glib::clone!(
                #[weak(rename_to = sidebar)]
                self,
                #[weak]
                list,
                move |_, _, x, y| {
                    sidebar.open_folder_menu(&list, x, y);
                }
            ));
            list.add_controller(folder_menu);
        }

        self.set_child(Some(&column));
        self.set_status(SyncStatus::default());
    }

    /// Opens a saved search's context menu exactly as a right-click would,
    /// registering its actions on `self` -- without simulating the click
    /// itself, which GTK4 gives a test no way to do (see #424, #437 for the
    /// same limit hit elsewhere in this crate). A test drives the result
    /// with `WidgetExt::activate_action("savedsearch.<verb>", None)`, the
    /// same portable entry point a real menu item's activation uses.
    #[doc(hidden)]
    pub fn test_open_saved_search_menu(&self, x: f64, y: f64) {
        self.open_saved_search_menu(x, y);
    }

    /// Closes the saved-search context menu, if one is open.
    ///
    /// `WidgetExt::activate_action` triggers a `SimpleAction`'s own
    /// callback directly, which is what `test_open_saved_search_menu`'s own
    /// doc says it is for -- but a *real* click also closes a
    /// `GtkPopoverMenu` as part of its built-in item-activation handling,
    /// and calling the action directly skips that entirely. A test that
    /// never calls this leaves a grabbing popover attached to the window
    /// through `window.destroy()`, which is exactly the hang this exists to
    /// prevent -- found by hitting it, not by reading ahead.
    #[doc(hidden)]
    pub fn test_close_saved_search_menu(&self) {
        if let Some(popover) = self.imp().saved_search_menu.take() {
            popover.popdown();
        }
    }

    /// Open a saved search's context menu at `(x, y)`, if there is a row
    /// there to open one for.
    ///
    /// Not generated from the command registry the way the message list's
    /// context menu is (`list_view.rs::open_context_menu`): these verbs
    /// have no keyboard path yet, since saved searches do not participate
    /// in `step`/`rows` keyboard navigation the way ordinary folders do --
    /// `postio_core::Context::Sidebar`'s existing model is keyed to
    /// `MailboxId`, and a saved search does not have one. A menu entry with
    /// no binding and no cheat-sheet line would be a lie about what the
    /// registry promises, so this is deliberately its own small, fixed
    /// menu rather than a registry-generated one -- see the issue for the
    /// keyboard-reachability gap this leaves, tracked separately.
    fn open_saved_search_menu(&self, x: f64, y: f64) {
        let imp = self.imp();
        if let Some(previous) = imp.saved_search_menu.take() {
            previous.popdown();
        }
        let Some(row) = imp.saved.row_at_y(y as i32) else {
            return;
        };
        let key = row_search_key(&row);
        if key.is_empty() {
            return;
        }
        let index = row.index();
        let is_first = index == 0;
        let is_last = imp.saved.row_at_index(index + 1).is_none();

        let menu = gtk::gio::Menu::new();
        menu.append(Some("Rename"), Some("savedsearch.rename"));
        if !is_first {
            menu.append(Some("Move up"), Some("savedsearch.move-up"));
        }
        if !is_last {
            menu.append(Some("Move down"), Some("savedsearch.move-down"));
        }
        menu.append(Some("Delete"), Some("savedsearch.delete"));

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(&imp.saved);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        // Only the actions the menu actually offers: a `SimpleAction` this
        // widget registered but no item pointed at would still be reachable
        // through `WidgetExt::activate_action`, existing despite nothing on
        // screen that reaches it -- so the same `is_first`/`is_last` gate
        // above decides both, not only what the menu draws.
        let actions = gtk::gio::SimpleActionGroup::new();
        let mut wanted = vec![
            ("rename", SavedSearchAction::Rename),
            ("delete", SavedSearchAction::Delete),
        ];
        if !is_first {
            wanted.push(("move-up", SavedSearchAction::MoveUp));
        }
        if !is_last {
            wanted.push(("move-down", SavedSearchAction::MoveDown));
        }
        for (name, action) in wanted {
            let simple = gtk::gio::SimpleAction::new(name, None);
            simple.connect_activate(glib::clone!(
                #[weak(rename_to = sidebar)]
                self,
                #[strong]
                key,
                move |_, _| {
                    for callback in sidebar.imp().saved_search_action.borrow().iter() {
                        callback(key.clone(), action);
                    }
                }
            ));
            actions.add_action(&simple);
        }
        // On `self`, not `imp.saved`: the popover's items resolve the action
        // by walking up from wherever they are clicked, and a group inserted
        // higher up still reaches them -- and doing it here, rather than on
        // a private field, is what lets `test_open_saved_search_menu` be
        // driven with `WidgetExt::activate_action` on the public `Sidebar`.
        self.insert_action_group("savedsearch", Some(&actions));

        popover.connect_closed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            popover,
            move |_| {
                popover.unparent();
                // Only if it is still the current one: `open_saved_search_menu`
                // itself already took it out when replacing it with a new
                // one, and `connect_closed` still fires for the outgoing
                // popover after that -- clearing an unrelated, already-
                // current popover here would be the bug this guards against.
                let current = sidebar.imp().saved_search_menu.borrow().clone();
                if current.as_ref() == Some(&popover) {
                    sidebar.imp().saved_search_menu.take();
                }
            }
        ));
        *imp.saved_search_menu.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    /// Opens an ordinary folder row's context menu exactly as a right-click
    /// on it would, registering its action on `self` -- the same shape
    /// `test_open_saved_search_menu` uses, and for the same reason (GTK4
    /// gives a test no way to simulate the click itself; see #424, #437).
    ///
    /// Takes the row rather than a click point: unlike the saved-search
    /// list, a `GtkListBoxRow`'s allocation is not dependably queryable
    /// through `row_at_y`/`compute_bounds` in the headless test compositor
    /// this crate's tests run under, and a test that already has the row
    /// (from walking the tree for it, the way every test here does) loses
    /// nothing by handing it over directly.
    #[doc(hidden)]
    pub fn test_open_ordinary_folder_menu(&self, row: &gtk::ListBoxRow) {
        let list = self.imp().ordinary.clone();
        self.open_folder_menu_for_row(&list, row);
    }

    /// As [`Sidebar::test_open_ordinary_folder_menu`], for a row in the
    /// special-use section instead — Inbox, Sent, Junk and the rest, which
    /// live in their own `GtkListBox` (see `sections`).
    #[doc(hidden)]
    pub fn test_open_special_folder_menu(&self, row: &gtk::ListBoxRow) {
        let list = self.imp().special.clone();
        self.open_folder_menu_for_row(&list, row);
    }

    /// Closes the folder context menu, if one is open — see
    /// `test_close_saved_search_menu` for why a test must call this rather
    /// than letting `window.destroy()` do it.
    #[doc(hidden)]
    pub fn test_close_folder_menu(&self) {
        if let Some(popover) = self.imp().folder_menu.take() {
            popover.popdown();
        }
    }

    /// Skip/resume background backfill for one folder (ADR 0016, #350): a
    /// single toggling entry, labelled for whichever direction it currently
    /// offers, at `(x, y)` in `list` if there is a folder row there.
    ///
    /// Takes `list` explicitly rather than assuming the ordinary tree: the
    /// special-use section (`imp.special`) is a separate `GtkListBox` with
    /// its own coordinate space, and ADR 0016's own motivating example,
    /// Junk, lives there.
    ///
    /// Not generated from the command registry the way the message list's
    /// context menu is: this verb has no keyboard path (it is a settings
    /// toggle, not a message action), so a menu entry with no binding and no
    /// cheat-sheet line is not a lie about what the registry promises here —
    /// there is no registry entry to disagree with.
    fn open_folder_menu(&self, list: &gtk::ListBox, x: f64, y: f64) {
        let Some(row) = list.row_at_y(y as i32) else {
            return;
        };
        self.open_folder_menu_at(list, &row, x, y);
    }

    /// As [`Sidebar::open_folder_menu`], with the row already resolved --
    /// what the two `test_open_*_folder_menu` methods use, since neither a
    /// click point nor the row's own allocation can be trusted to answer
    /// `row_at_y` reliably outside a real, fully laid-out window (see their
    /// doc comments). `(1.0, 0.0)` stands in for a click point a test has
    /// none of; the popover's position is not part of what either checks.
    fn open_folder_menu_for_row(&self, list: &gtk::ListBox, row: &gtk::ListBoxRow) {
        self.open_folder_menu_at(list, row, 1.0, 0.0);
    }

    fn open_folder_menu_at(&self, list: &gtk::ListBox, row: &gtk::ListBoxRow, x: f64, y: f64) {
        let imp = self.imp();
        if let Some(previous) = imp.folder_menu.take() {
            previous.popdown();
        }
        let id = MailboxId::new(row_id(row));
        let Some(mailbox) = imp.mailboxes.borrow().iter().find(|m| m.id == id).cloned() else {
            return;
        };
        let excluded = mailbox.backfill_excluded;

        let menu = gtk::gio::Menu::new();
        menu.append(
            Some(if excluded {
                "Resume backing up locally"
            } else {
                "Skip backing up locally"
            }),
            Some("folder.toggle-backfill"),
        );

        let popover = gtk::PopoverMenu::from_model(Some(&menu));
        popover.set_parent(list);
        popover.set_has_arrow(false);
        popover.set_halign(gtk::Align::Start);
        popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, y as i32, 1, 1)));

        let actions = gtk::gio::SimpleActionGroup::new();
        let toggle = gtk::gio::SimpleAction::new("toggle-backfill", None);
        toggle.connect_activate(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            move |_, _| {
                for callback in sidebar.imp().backfill_exclusion_changed.borrow().iter() {
                    callback(id, !excluded);
                }
            }
        ));
        actions.add_action(&toggle);
        // On `self`, for the same reason `open_saved_search_menu` does:
        // `test_open_ordinary_folder_menu`/`test_open_special_folder_menu`
        // drive the result with
        // `WidgetExt::activate_action("folder.toggle-backfill", None)`.
        self.insert_action_group("folder", Some(&actions));

        popover.connect_closed(glib::clone!(
            #[weak(rename_to = sidebar)]
            self,
            #[weak]
            popover,
            move |_| {
                popover.unparent();
                let current = sidebar.imp().folder_menu.borrow().clone();
                if current.as_ref() == Some(&popover) {
                    sidebar.imp().folder_menu.take();
                }
            }
        ));
        *imp.folder_menu.borrow_mut() = Some(popover.clone());
        popover.popup();
    }

    /// Called when a folder's context menu flips whether it participates in
    /// background backfill (ADR 0016, #350), with the mailbox and what it
    /// should become. Persisting the change and calling
    /// [`Sidebar::set_mailboxes`] with the result is the caller's job, the
    /// same split [`Sidebar::connect_saved_search_action`] already draws.
    pub fn connect_backfill_exclusion_changed(&self, handler: impl Fn(MailboxId, bool) + 'static) {
        self.imp()
            .backfill_exclusion_changed
            .borrow_mut()
            .push(Box::new(handler));
    }

    /// The account address shown at the top.
    pub fn set_account(&self, address: &str) {
        self.imp().account.set_text(address);
    }

    /// The mailbox list [`Sidebar::set_mailboxes`] was last given.
    pub fn mailboxes(&self) -> Vec<Mailbox> {
        self.imp().mailboxes.borrow().clone()
    }

    /// Replace the folder list.
    ///
    /// Rows for mailboxes that are still there are updated in place rather
    /// than rebuilt, so a count changing does not take the selection or the
    /// keyboard focus with it — which is the whole of "counts update live".
    pub fn set_mailboxes(&self, mailboxes: &[Mailbox]) {
        let imp = self.imp();
        *imp.mailboxes.borrow_mut() = mailboxes.to_vec();
        let (special, _) = sections(mailboxes);
        let selected = self.selected();

        sync_rows(&imp.special, &special, self);
        self.render_ordinary();

        if let Some(id) = selected {
            self.select(id);
        }
    }

    /// Rebuild the ordinary section from the cached mailbox list and the
    /// current collapse state (#324). Called after `set_mailboxes`, and
    /// after anything that changes what is collapsed.
    fn render_ordinary(&self) {
        let imp = self.imp();
        let rows = folder_rows(&imp.mailboxes.borrow(), &imp.collapsed.borrow());
        sync_folder_rows(&imp.ordinary, &rows, self);
        imp.ordinary_section.set_visible(!rows.is_empty());
    }

    /// Expand or collapse `id`'s children. A harmless no-op if `id` has none
    /// — the caller does not have to check first.
    pub fn toggle(&self, id: MailboxId) {
        {
            let mut collapsed = self.imp().collapsed.borrow_mut();
            if !collapsed.remove(&id) {
                collapsed.insert(id);
            }
        }
        self.render_ordinary();
        self.notify_collapsed_changed();
    }

    /// Toggle whichever folder is currently selected: the keyboard's
    /// `toggle_folder` command acts on it, because selection is already the
    /// sidebar's own notion of "where the keyboard is" — see
    /// [`Sidebar::step`], which selects and opens in the same motion.
    pub fn toggle_focused(&self) {
        if let Some(id) = self.selected() {
            self.toggle(id);
        }
    }

    /// Set which folders are showing collapsed — what a caller restoring
    /// saved state calls before or after the first [`Sidebar::set_mailboxes`].
    pub fn set_collapsed(&self, collapsed: HashSet<MailboxId>) {
        *self.imp().collapsed.borrow_mut() = collapsed;
        self.render_ordinary();
    }

    /// The folders currently showing collapsed, for a caller to persist.
    pub fn collapsed(&self) -> HashSet<MailboxId> {
        self.imp().collapsed.borrow().clone()
    }

    /// Called whenever [`Sidebar::toggle`] or an ancestor auto-expand (see
    /// [`Sidebar::select`]) changes which folders are collapsed.
    pub fn connect_collapsed_changed(&self, callback: impl Fn() + 'static) {
        self.imp()
            .collapsed_changed
            .borrow_mut()
            .push(Box::new(callback));
    }

    fn notify_collapsed_changed(&self) {
        for handler in self.imp().collapsed_changed.borrow().iter() {
            handler();
        }
    }

    /// Replace the saved-search list, in exactly the order given.
    ///
    /// The caller's order, not this widget's: alphabetical was fine while
    /// nothing else was possible, but once a saved search can be reordered
    /// (#292, `Config::ordered_filter_keys`) re-sorting here would silently
    /// throw that away every time the sidebar redraws. Rows are updated in
    /// place, exactly as [`Sidebar::set_mailboxes`] does, so a rename or a
    /// requery does not cost the selection.
    pub fn set_saved_searches(&self, searches: &[SavedSearch]) {
        let imp = self.imp();
        sync_search_rows(&imp.saved, searches);
        imp.saved_section.set_visible(!searches.is_empty());
    }

    /// The selected folder, if any.
    pub fn selected(&self) -> Option<MailboxId> {
        let imp = self.imp();
        for list in [&imp.special, &imp.ordinary] {
            if let Some(row) = list.selected_row() {
                return Some(MailboxId::new(row_id(&row)));
            }
        }
        None
    }

    /// Select a folder without reporting it back as a user action.
    ///
    /// Opens every collapsed ancestor first (#324): a folder selected while
    /// its parent is closed must still show as selected, rather than
    /// silently landing on no row at all because the one it belongs to is
    /// not currently drawn.
    pub fn select(&self, id: MailboxId) {
        let imp = self.imp();
        let ancestors = ancestors_of(&imp.mailboxes.borrow(), id);
        let opened = {
            let mut collapsed = imp.collapsed.borrow_mut();
            // `|=`, not `||=`, so removing an ancestor is never short-circuited
            // away by an earlier one already having opened something.
            let mut any = false;
            for ancestor in &ancestors {
                any |= collapsed.remove(ancestor);
            }
            any
        };
        if opened {
            self.render_ordinary();
            self.notify_collapsed_changed();
        }

        imp.echoing.set(true);
        for list in [&imp.special, &imp.ordinary] {
            match find_row(list, id) {
                Some(row) => list.select_row(Some(&row)),
                None => list.unselect_all(),
            }
        }
        imp.echoing.set(false);
    }

    /// Every folder row, both sections, in the order they are drawn.
    ///
    /// The two `GtkListBox`es are a *visual* split — special-use folders,
    /// then a rule, then the rest — and the keyboard must not know about it.
    /// `j` at the bottom of the first section goes to the top of the second,
    /// because that is the next folder on screen.
    fn rows(&self) -> Vec<gtk::ListBoxRow> {
        let imp = self.imp();
        let mut rows = Vec::new();
        for list in [&imp.special, &imp.ordinary] {
            let mut index = 0;
            while let Some(row) = list.row_at_index(index) {
                rows.push(row);
                index += 1;
            }
        }
        rows
    }

    /// Put the keyboard in the folder list.
    ///
    /// On the selected folder, or the first one when nothing is selected —
    /// never nowhere. Returns whether there was a folder to land on, so the
    /// caller can leave the keyboard where it was rather than sending it into
    /// an empty pane on a first run.
    pub fn focus_folders(&self) -> bool {
        let rows = self.rows();
        let landing = self
            .selected()
            .and_then(|id| rows.iter().find(|row| MailboxId::new(row_id(row)) == id))
            .or_else(|| rows.first());
        match landing {
            Some(row) => {
                row.grab_focus();
                true
            }
            None => false,
        }
    }

    /// Move the selection `delta` rows and report the folder landed on.
    ///
    /// Selection *is* the open folder here, exactly as it is for the mouse: a
    /// click selects and opens, so `j` selects and opens. Making the keyboard
    /// move a cursor that has to be confirmed would be a second idiom for
    /// the same pane.
    ///
    /// Stops at the ends rather than wrapping. Wrapping a short list is how
    /// you end up in Trash when you meant to stop at Inbox.
    pub fn step(&self, delta: i32) -> Option<MailboxId> {
        let rows = self.rows();
        if rows.is_empty() {
            return None;
        }
        let current = self.selected().and_then(|id| {
            rows.iter()
                .position(|row| MailboxId::new(row_id(row)) == id)
        });
        let next = match current {
            Some(index) => (index as i32 + delta).clamp(0, rows.len() as i32 - 1) as usize,
            // Nothing selected: `j` starts at the top and `k` at the bottom,
            // so both keys reach a folder from a standing start.
            None if delta > 0 => 0,
            None => rows.len() - 1,
        };
        let row = &rows[next];
        row.grab_focus();
        let id = MailboxId::new(row_id(row));
        self.select(id);
        // A `\Noselect` container has nothing to open: stepping onto it
        // moves the keyboard there — so `toggle_focused` (#324) has
        // something to act on — but must not report it as an open folder,
        // the same gate the click handler applies.
        if self.is_openable(id) {
            // `select` is deliberately quiet — it is what the window calls
            // to echo a folder it opened — so the keyboard has to announce
            // its own move, the same way a click does.
            for handler in self.imp().selected.borrow().iter() {
                handler(id);
            }
        }
        Some(id)
    }

    /// Whether `id` names a folder that can actually be opened — false for a
    /// `\Noselect` container, and true for an id this sidebar has never
    /// heard of, so an unrelated caller's id is not silently swallowed.
    fn is_openable(&self, id: MailboxId) -> bool {
        self.imp()
            .mailboxes
            .borrow()
            .iter()
            .find(|m| m.id == id)
            .is_none_or(|m| m.selectable)
    }

    /// Called when the user picks a folder, by click or by keyboard.
    /// Called when messages are dropped on a folder.
    ///
    /// The move itself is not this widget's to make: it hands over what was
    /// dropped and where, and the window turns that into the registry's
    /// `Move` command so a drag and the `m` key are the same action.
    pub fn connect_dropped(
        &self,
        callback: impl Fn(crate::list_view::Dragged, MailboxId) + 'static,
    ) {
        self.imp().dropped.borrow_mut().push(Box::new(callback));
    }

    pub fn connect_selected(&self, callback: impl Fn(MailboxId) + 'static) {
        self.imp().selected.borrow_mut().push(Box::new(callback));
    }

    /// What to call when the user picks a saved search: its query, not its
    /// name -- running it is the caller's job, and the query is all that
    /// takes.
    pub fn connect_search_selected(&self, callback: impl Fn(String) + 'static) {
        self.imp()
            .search_selected
            .borrow_mut()
            .push(Box::new(callback));
    }

    /// What to call when a saved search's context menu picks
    /// [`SavedSearchAction::Rename`], `MoveUp`, `MoveDown` or `Delete`, with
    /// the `[filters.<key>]` key the row was drawn from.
    ///
    /// The widget knows nothing about `postio-config` -- it hands back the
    /// key and the verb, and asking `Config` to act on them, saving, and
    /// calling [`Sidebar::set_saved_searches`] with the result is the
    /// caller's job, the same split `connect_search_selected` already
    /// draws.
    pub fn connect_saved_search_action(
        &self,
        callback: impl Fn(String, SavedSearchAction) + 'static,
    ) {
        self.imp()
            .saved_search_action
            .borrow_mut()
            .push(Box::new(callback));
    }

    /// Replace what the status line says.
    pub fn set_status(&self, status: SyncStatus) {
        let imp = self.imp();
        *imp.status.borrow_mut() = status;
        self.render_status();
    }

    /// The status as it stands.
    pub fn status(&self) -> SyncStatus {
        self.imp().status.borrow().clone()
    }

    fn render_status(&self) {
        let imp = self.imp();
        let status = imp.status.borrow().clone();
        let now = Instant::now();
        let (state, detail) = status.lines(now);
        imp.status_state.set_text(&state);
        imp.status_detail.set_text(&detail);
        set_class(
            &imp.status_state,
            "error",
            matches!(status.state, ConnectionState::Failing { .. }),
        );

        // Re-arm at the granularity the line is actually showing, so an age in
        // days does not wake the process up every second.
        if let Some(tick) = imp.tick.borrow_mut().take() {
            tick.remove();
        }
        if let Some(interval) = status.refresh_interval(now) {
            let source = glib::timeout_add_local(
                interval,
                glib::clone!(
                    #[weak(rename_to = sidebar)]
                    self,
                    #[upgrade_or]
                    glib::ControlFlow::Break,
                    move || {
                        sidebar.render_status();
                        glib::ControlFlow::Break
                    }
                ),
            );
            *imp.tick.borrow_mut() = Some(source);
        }
    }
}

/// A scrolling list of folder rows.
fn folder_list(list: &gtk::ListBox) -> gtk::Widget {
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.add_css_class("postio-folders");
    list.upcast_ref::<gtk::Widget>().clone()
}

/// A list of saved-search rows, the same shape as [`folder_list`].
fn search_list(list: &gtk::ListBox) -> gtk::Widget {
    list.set_selection_mode(gtk::SelectionMode::Single);
    list.upcast_ref::<gtk::Widget>().clone()
}

/// Bring `list` in line with `mailboxes`, reusing the rows that survive.
fn sync_rows(list: &gtk::ListBox, mailboxes: &[Mailbox], sidebar: &Sidebar) {
    for (index, mailbox) in mailboxes.iter().enumerate() {
        match list.row_at_index(index as i32) {
            Some(row) => update_row(&row, mailbox),
            None => {
                let row = folder_row(mailbox);
                accept_drops(&row, sidebar);
                list.append(&row);
            }
        }
    }
    while let Some(extra) = list.row_at_index(mailboxes.len() as i32) {
        list.remove(&extra);
    }
}

/// `name  count`, at the canvas' 36px.
fn folder_row(mailbox: &Mailbox) -> gtk::ListBoxRow {
    let name = gtk::Label::new(None);
    name.add_css_class("postio-folder-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);

    let count = gtk::Label::new(None);
    count.add_css_class("postio-folder-count");

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    line.append(&name);
    line.append(&count);

    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-folder");
    row.set_child(Some(&line));
    update_row(&row, mailbox);
    row
}

/// Make `row` a place messages can be dropped.
///
/// A folder is the one thing in the sidebar a message can be moved *to*, so
/// every folder row is a target and nothing else is. The highlight is the
/// whole point: a drop target that does not say it is one is a drop that
/// happens by luck.
fn accept_drops(row: &gtk::ListBoxRow, sidebar: &Sidebar) {
    let target = gtk::DropTarget::new(glib::types::Type::STRING, gtk::gdk::DragAction::MOVE);

    target.connect_enter(glib::clone!(
        #[weak]
        sidebar,
        #[upgrade_or]
        gtk::gdk::DragAction::empty(),
        move |target, _, _| {
            let Some(row) = target.widget().and_downcast::<gtk::ListBoxRow>() else {
                return gtk::gdk::DragAction::empty();
            };
            // SAFETY: the key is private to this module and always holds an i64.
            let mailbox = MailboxId::new(row_id(&row));
            // The folder the mail is already in says no rather than saying
            // nothing: a target that highlights and then does nothing is
            // worse than one that never lit up.
            if sidebar.selected() == Some(mailbox) {
                return gtk::gdk::DragAction::empty();
            }
            row.add_css_class("postio-drop-into");
            gtk::gdk::DragAction::MOVE
        }
    ));
    target.connect_leave(|target| {
        if let Some(row) = target.widget() {
            row.remove_css_class("postio-drop-into");
        }
    });
    target.connect_drop(glib::clone!(
        #[weak]
        sidebar,
        #[upgrade_or]
        false,
        move |target, value, _, _| {
            if let Some(row) = target.widget() {
                row.remove_css_class("postio-drop-into");
            }
            let Some(row) = target.widget().and_downcast::<gtk::ListBoxRow>() else {
                return false;
            };
            let Ok(payload) = value.get::<String>() else {
                return false;
            };
            let Some(dragged) = crate::list_view::dragged_messages(&payload) else {
                return false;
            };
            // SAFETY: the key is private to this module and always holds an i64.
            let mailbox = MailboxId::new(row_id(&row));
            // Dropping mail into the folder it is already in is not a move,
            // and reporting it as one would put an undo entry on the stack
            // for something that did not happen.
            if sidebar.selected() == Some(mailbox) {
                return false;
            }
            for handler in sidebar.imp().dropped.borrow().iter() {
                handler(dragged.clone(), mailbox);
            }
            true
        }
    ));
    row.add_controller(target);
}

fn update_row(row: &gtk::ListBoxRow, mailbox: &Mailbox) {
    // SAFETY: the key is private to this module and always holds an i64.
    // The only writer of this key; `row_id` is the only reader. Keeping both
    // in this file is what lets `row_id` be safe.
    #[allow(unsafe_code)]
    unsafe {
        row.set_data("postio-mailbox-id", mailbox.id.get())
    };

    let Some(line) = row.child().and_then(|c| c.downcast::<gtk::Box>().ok()) else {
        return;
    };
    let Some(name) = line
        .first_child()
        .and_then(|c| c.downcast::<gtk::Label>().ok())
    else {
        return;
    };
    let Some(count) = name
        .next_sibling()
        .and_then(|c| c.downcast::<gtk::Label>().ok())
    else {
        return;
    };

    // The empty slice reads as "no rival": every mailbox `sections` puts in
    // the special section is a primary by construction, so the role name is
    // always the right answer here.
    name.set_text(&display_name(mailbox, &[]));
    match count_for(mailbox) {
        Some(value) => {
            count.set_text(&value.to_string());
            count.set_visible(true);
        }
        None => count.set_visible(false),
    }

    // The row announces both halves, and says what the number *is*. Sighted
    // readers get that from the column it sits in; a screen reader given
    // "Inbox, 12" is given a bare number, and the same digit means unread in
    // one folder and total in another — see `count_for`.
    row.update_property(&[gtk::accessible::Property::Label(&announce(
        &display_name(mailbox, &[]),
        mailbox,
    ))]);
}

/// What a screen reader says for a folder row, given the name it is drawn
/// under — the role name in the special section, the server's own name in
/// the tree (#501).
fn announce(name: &str, mailbox: &Mailbox) -> String {
    let name = name.to_string();
    let Some(count) = count_for(mailbox) else {
        return name;
    };
    match mailbox.role {
        MailboxRole::Drafts => format!("{name}, {count} drafts"),
        MailboxRole::Flagged => format!("{name}, {count} flagged"),
        _ => format!("{name}, {count} unread"),
    }
}

/// What a folder is called in the sidebar.
///
/// The special-use folders get the name Postio uses for the role, not the one
/// the server happens to have picked: an iCloud account calls its archive
/// "Archive" but its junk folder "Junk E-mail", and the sidebar is not the
/// place to learn that.
///
/// Public because the list pane's header names the same folder, and two
/// places calling one mailbox by two names is exactly the vocabulary drift
/// this function exists to prevent.
pub fn display_name(mailbox: &Mailbox, among: &[Mailbox]) -> String {
    if !primary_within(mailbox, among) {
        // The role's *twin* (#501): a second folder the server reports with
        // the same role. It renders as an ordinary folder, and an ordinary
        // folder is called what the server calls it — the role name belongs
        // to exactly one row, or the sidebar reads `Sent, Sent`.
        return mailbox.name.clone();
    }
    match mailbox.role {
        MailboxRole::Inbox => "Inbox".to_string(),
        MailboxRole::Flagged => "Flagged".to_string(),
        MailboxRole::Drafts => "Drafts".to_string(),
        MailboxRole::Sent => "Sent".to_string(),
        MailboxRole::Archive => "Archive".to_string(),
        MailboxRole::Junk => "Junk".to_string(),
        MailboxRole::Trash => "Trash".to_string(),
        MailboxRole::Regular => mailbox.name.clone(),
    }
}

/// Whether `mailbox` is the folder its role actually routes to, among its
/// account's mailboxes.
///
/// The same answer `MailboxRepository::by_role` gives — first by path — so
/// the folder the sidebar crowns with the role name is the folder `a`
/// archives into and `d` deletes into. Two rules diverging here is how a
/// sidebar says `Archive` over one folder while the key files into another.
///
/// A role-less mailbox is trivially primary: there is nothing to be the
/// twin of.
pub fn primary_within(mailbox: &Mailbox, among: &[Mailbox]) -> bool {
    if role_order(mailbox.role).is_none() {
        return true;
    }
    // Identity by path, not id: paths are unique within an account and are
    // what `by_role` orders by, while ids are storage rowids a fixture never
    // sets.
    !among.iter().any(|other| {
        other.account_id == mailbox.account_id
            && other.role == mailbox.role
            && other.path < mailbox.path
    })
}

/// Pixels of indent per nesting level (#324). Not a design token — this
/// row's shape is laid out in code, not CSS, because how far in it sits is
/// data (`FolderRow::depth`), not something a stylesheet selector can see.
const INDENT_PX: i32 = 16;

/// Bring `list` in line with `rows`, reusing the rows that survive — the
/// ordinary section's own counterpart of [`sync_rows`], which has a
/// hierarchy to draw that the special-use section does not.
fn sync_folder_rows(list: &gtk::ListBox, rows: &[FolderRow], sidebar: &Sidebar) {
    for (index, data) in rows.iter().enumerate() {
        match list.row_at_index(index as i32) {
            Some(row) => update_tree_row(&row, data, sidebar),
            None => {
                let row = tree_row(data, sidebar);
                accept_drops(&row, sidebar);
                list.append(&row);
            }
        }
    }
    while let Some(extra) = list.row_at_index(rows.len() as i32) {
        list.remove(&extra);
    }
}

/// `[indent][disclosure] name  count` — structure is [`update_tree_row`]'s
/// promise to keep in step, exactly as [`folder_row`]'s is for the flat
/// special-use rows.
fn tree_row(data: &FolderRow, sidebar: &Sidebar) -> gtk::ListBoxRow {
    let indent = gtk::Box::new(gtk::Orientation::Horizontal, 0);

    let disclosure = gtk::Button::from_icon_name("pan-end-symbolic");
    disclosure.add_css_class("flat");
    disclosure.add_css_class("postio-folder-disclosure");
    disclosure.set_valign(gtk::Align::Center);

    let name = gtk::Label::new(None);
    name.add_css_class("postio-folder-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);

    let count = gtk::Label::new(None);
    count.add_css_class("postio-folder-count");

    let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    line.append(&indent);
    line.append(&disclosure);
    line.append(&name);
    line.append(&count);

    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-folder");
    row.add_css_class("postio-folder-tree");
    row.set_child(Some(&line));

    // The disclosure is its own widget with its own click gesture, so
    // clicking it does not also select the row — the same reason a switch
    // or a button embedded in any GTK list row works today.
    disclosure.connect_clicked(glib::clone!(
        #[weak]
        sidebar,
        #[weak]
        row,
        move |_| sidebar.toggle(MailboxId::new(row_id(&row)))
    ));

    update_tree_row(&row, data, sidebar);
    row
}

fn update_tree_row(row: &gtk::ListBoxRow, data: &FolderRow, sidebar: &Sidebar) {
    // SAFETY: as `update_row` — the only writer of this key on a tree row.
    // `row_id` reads it back without caring which kind of row wrote it.
    #[allow(unsafe_code)]
    unsafe {
        row.set_data("postio-mailbox-id", data.mailbox.id.get())
    };

    let Some(line) = row.child().and_then(|c| c.downcast::<gtk::Box>().ok()) else {
        return;
    };
    let Some(indent) = line
        .first_child()
        .and_then(|c| c.downcast::<gtk::Box>().ok())
    else {
        return;
    };
    let Some(disclosure) = indent
        .next_sibling()
        .and_then(|c| c.downcast::<gtk::Button>().ok())
    else {
        return;
    };
    let Some(name) = disclosure
        .next_sibling()
        .and_then(|c| c.downcast::<gtk::Label>().ok())
    else {
        return;
    };
    let Some(count) = name
        .next_sibling()
        .and_then(|c| c.downcast::<gtk::Label>().ok())
    else {
        return;
    };

    indent.set_size_request(data.depth as i32 * INDENT_PX, -1);

    let collapsed = sidebar.collapsed().contains(&data.mailbox.id);
    disclosure.set_icon_name(if collapsed {
        "pan-end-symbolic"
    } else {
        "pan-down-symbolic"
    });
    // A row with nothing to disclose keeps the control's width, so every
    // row's name still lines up at its own depth, but it neither shows nor
    // takes a click or the keyboard's Tab order.
    disclosure.set_opacity(if data.has_children { 1.0 } else { 0.0 });
    disclosure.set_sensitive(data.has_children);
    disclosure.set_can_focus(data.has_children);
    disclosure.update_property(&[gtk::accessible::Property::Label(if collapsed {
        "Expand"
    } else {
        "Collapse"
    })]);

    // The server's own name, always: every role primary lives in the
    // special section, so a role-bearing mailbox down here is a twin
    // (#501), and calling it by its role would draw the second `Archive`
    // this section exists to avoid.
    name.set_text(&data.mailbox.name);
    match count_for(&data.mailbox) {
        Some(value) => {
            count.set_text(&value.to_string());
            count.set_visible(true);
        }
        None => count.set_visible(false),
    }

    row.update_property(&[gtk::accessible::Property::Label(&announce(
        &data.mailbox.name,
        &data.mailbox,
    ))]);
}

fn find_row(list: &gtk::ListBox, id: MailboxId) -> Option<gtk::ListBoxRow> {
    let mut index = 0;
    while let Some(row) = list.row_at_index(index) {
        if row_id(&row) == id.get() {
            return Some(row);
        }
        index += 1;
    }
    None
}

/// The mailbox id [`update_row`] stored on `row`, or 0 if it has none.
///
/// Safe, and the reason is a module invariant rather than a caller's promise:
/// `"postio-mailbox-id"` is private to this file and is only ever written by
/// [`update_row`], always as an `i64`. Nothing outside can put another type
/// under that key, so there is no obligation left for a caller to discharge --
/// which is what makes confining the `unsafe` here correct rather than
/// convenient. It used to be an `unsafe fn`, and its nine call sites each
/// opened an `unsafe` block to repeat an argument that was already true.
fn row_id(row: &gtk::ListBoxRow) -> i64 {
    // glib cannot know the type a key was stored under; this file can.
    #[allow(unsafe_code)]
    unsafe {
        row.data::<i64>("postio-mailbox-id")
            .map(|p| *p.as_ref())
            .unwrap_or_default()
    }
}

/// Bring `list` in line with `searches`, reusing the rows that survive --
/// the saved-search counterpart of [`sync_rows`].
fn sync_search_rows(list: &gtk::ListBox, searches: &[SavedSearch]) {
    for (index, search) in searches.iter().enumerate() {
        match list.row_at_index(index as i32) {
            Some(row) => update_search_row(&row, search),
            None => list.append(&search_row(search)),
        }
    }
    while let Some(extra) = list.row_at_index(searches.len() as i32) {
        list.remove(&extra);
    }
}

/// One saved-search row: its name, nothing beside it.
///
/// No count the way a folder has one: a saved search's number would be how
/// many messages it currently matches, which costs a query to know and is
/// not what the sidebar promises to keep live for anything else in it.
fn search_row(search: &SavedSearch) -> gtk::ListBoxRow {
    let name = gtk::Label::new(None);
    name.add_css_class("postio-folder-name");
    name.set_xalign(0.0);
    name.set_hexpand(true);
    name.set_ellipsize(pango::EllipsizeMode::End);

    let row = gtk::ListBoxRow::new();
    row.add_css_class("postio-saved-search");
    row.set_child(Some(&name));
    update_search_row(&row, search);
    row
}

fn update_search_row(row: &gtk::ListBoxRow, search: &SavedSearch) {
    // SAFETY: both keys are private to this module and always hold a
    // `String`. The only writer of either; `row_query`/`row_search_key` are
    // the only readers.
    #[allow(unsafe_code)]
    unsafe {
        row.set_data("postio-search-query", search.query.clone());
        row.set_data("postio-search-key", search.key.clone());
    };
    if let Some(name) = row.child().and_then(|c| c.downcast::<gtk::Label>().ok()) {
        name.set_text(&search.name);
    }
    row.update_property(&[gtk::accessible::Property::Label(&search.name)]);
}

/// The query [`update_search_row`] stored on `row`, or empty if it has none.
///
/// Safe for the same reason [`row_id`] is: `"postio-search-query"` is
/// private to this file and is only ever written by [`update_search_row`],
/// always as a `String`.
fn row_query(row: &gtk::ListBoxRow) -> String {
    // glib cannot know the type a key was stored under; this file can.
    #[allow(unsafe_code)]
    unsafe {
        row.data::<String>("postio-search-query")
            .map(|p| p.as_ref().clone())
            .unwrap_or_default()
    }
}

/// The `[filters.<key>]` key [`update_search_row`] stored on `row` -- what
/// its context menu acts on, as opposed to [`row_query`]'s "what it runs".
fn row_search_key(row: &gtk::ListBoxRow) -> String {
    #[allow(unsafe_code)]
    unsafe {
        row.data::<String>("postio-search-key")
            .map(|p| p.as_ref().clone())
            .unwrap_or_default()
    }
}

fn set_class(widget: &impl IsA<gtk::Widget>, class: &str, on: bool) {
    if on {
        widget.as_ref().add_css_class(class);
    } else {
        widget.as_ref().remove_css_class(class);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use postio_model::ids::AccountId;
    use postio_model::mailbox::MailboxCounts;

    fn mailbox(path: &str, role: MailboxRole, counts: MailboxCounts) -> Mailbox {
        let mut mailbox = Mailbox::new(AccountId::new(1), path, Some('/'));
        mailbox.role = role;
        mailbox.counts = counts;
        mailbox
    }

    fn counts(total: u32, unread: u32, flagged: u32) -> MailboxCounts {
        MailboxCounts {
            total,
            unread,
            flagged,
        }
    }

    #[test]
    fn the_canvas_counts_come_out_of_the_canvas_folders() {
        // Canvas 1b: Inbox 12, Flagged 3, Drafts 2, Sent and Archive nothing.
        assert_eq!(
            count_for(&mailbox("INBOX", MailboxRole::Inbox, counts(940, 12, 3))),
            Some(12),
            "the inbox counts what you have not read"
        );
        assert_eq!(
            count_for(&mailbox(
                "Flagged",
                MailboxRole::Flagged,
                counts(940, 12, 3)
            )),
            Some(3),
            "flagged counts what is flagged, not what is unread"
        );
        assert_eq!(
            count_for(&mailbox("Drafts", MailboxRole::Drafts, counts(2, 0, 0))),
            Some(2),
            "a draft is never unread, so drafts count the lot"
        );
        assert_eq!(
            count_for(&mailbox("Sent", MailboxRole::Sent, counts(4000, 0, 0))),
            None
        );
        assert_eq!(
            count_for(&mailbox(
                "Archive",
                MailboxRole::Archive,
                counts(40000, 9, 0)
            )),
            None
        );
    }

    #[test]
    fn a_count_of_zero_is_not_drawn() {
        assert_eq!(
            count_for(&mailbox("INBOX", MailboxRole::Inbox, counts(940, 0, 0))),
            None,
            "an inbox with nothing unread shows no number at all"
        );
    }

    #[test]
    fn folders_split_into_the_canvas_two_sections() {
        let mailboxes = vec![
            mailbox("wayland-devel", MailboxRole::Regular, counts(37, 37, 0)),
            mailbox("Archive", MailboxRole::Archive, counts(1, 0, 0)),
            mailbox("INBOX", MailboxRole::Inbox, counts(1, 1, 0)),
            mailbox("lkml", MailboxRole::Regular, counts(204, 204, 0)),
            mailbox("Drafts", MailboxRole::Drafts, counts(2, 0, 0)),
            mailbox("Sent", MailboxRole::Sent, counts(9, 0, 0)),
            mailbox("Flagged", MailboxRole::Flagged, counts(3, 0, 3)),
        ];

        let (special, ordinary) = sections(&mailboxes);
        let names: Vec<String> = special
            .iter()
            .map(|m| display_name(m, &mailboxes))
            .collect();
        assert_eq!(
            names,
            ["Inbox", "Flagged", "Drafts", "Sent", "Archive"],
            "the canvas' order, whatever order the server listed them in"
        );

        let names: Vec<&str> = ordinary.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(
            names,
            ["lkml", "wayland-devel"],
            "ordinary folders sort by path"
        );
    }

    #[test]
    fn one_row_per_role_and_the_rest_keep_their_server_names() {
        // #501: an account that has passed through more than one client
        // really does hold two folders per role — `Sent` and `Sent
        // Messages`, `Archive` and `Archives`, `Deleted Messages` and
        // `Trash` — and renaming every role-bearing folder to its role drew
        // `Sent, Sent, Archive, Archive, Trash, Trash`: six rows, three
        // names, no way to tell which is which.
        //
        // The rule: the special section holds exactly one row per role —
        // the same mailbox `MailboxRepository::by_role` routes actions to,
        // first by path — and every other role-bearing folder is an
        // ordinary folder under its own server name, children intact.
        let mailboxes = vec![
            mailbox("INBOX", MailboxRole::Inbox, counts(40, 40, 0)),
            mailbox("Archive", MailboxRole::Archive, counts(0, 0, 0)),
            mailbox("Archives", MailboxRole::Archive, counts(1200, 0, 0)),
            mailbox("Sent", MailboxRole::Sent, counts(9, 0, 0)),
            mailbox("Sent Messages", MailboxRole::Sent, counts(400, 0, 0)),
            mailbox("Deleted Messages", MailboxRole::Trash, counts(3, 0, 0)),
            mailbox("Trash", MailboxRole::Trash, counts(7, 0, 0)),
        ];

        let (special, ordinary) = sections(&mailboxes);
        let names: Vec<String> = special
            .iter()
            .map(|m| display_name(m, &mailboxes))
            .collect();
        assert_eq!(
            names,
            ["Inbox", "Sent", "Archive", "Trash"],
            "one row per role: the mailbox actions route to, and no twin"
        );
        assert_eq!(
            special
                .iter()
                .map(|m| m.path.as_str())
                .collect::<Vec<_>>(),
            ["INBOX", "Sent", "Archive", "Deleted Messages"],
            "the primary is by_role's own answer: first by path"
        );

        let names: Vec<String> = ordinary
            .iter()
            .map(|m| display_name(m, &mailboxes))
            .collect();
        assert_eq!(
            names,
            ["Archives", "Sent Messages", "Trash"],
            "the twins are ordinary folders under their server names"
        );
    }

    #[test]
    fn a_child_of_a_twin_special_folder_nests_under_it() {
        // The other half of #501: `2024`, `2025`, `2026` filed under
        // `Archives` rendered as roots in FOLDERS, disowned by the Archive
        // row two inches above them, because the tree only ever considered
        // role-less mailboxes.
        let mut archives = mailbox("Archives", MailboxRole::Archive, counts(0, 0, 0));
        archives.id = MailboxId::new(2);
        let mut y2024 = mailbox("Archives/2024", MailboxRole::Regular, counts(0, 0, 0));
        y2024.id = MailboxId::new(3);
        y2024.parent_id = Some(archives.id);
        let mut archive = mailbox("Archive", MailboxRole::Archive, counts(0, 0, 0));
        archive.id = MailboxId::new(1);
        let mailboxes = vec![archive, archives, y2024];

        let rows = folder_rows(&mailboxes, &HashSet::new());
        let described: Vec<(String, u8)> = rows
            .iter()
            .map(|row| (row.mailbox.name.clone(), row.depth))
            .collect();
        assert_eq!(
            described,
            [("Archives".to_string(), 0), ("2024".to_string(), 1)],
            "the twin is in the tree and its children nest under it"
        );
    }

    #[test]
    fn a_folder_you_cannot_open_is_not_listed() {
        let mut container = mailbox("Lists", MailboxRole::Regular, counts(0, 0, 0));
        container.selectable = false;
        let (special, ordinary) = sections(&[container]);
        assert!(special.is_empty() && ordinary.is_empty());
    }

    /// A mailbox with an explicit id and parent, for tree tests — `mailbox`
    /// above leaves every id `UNASSIGNED`, which is fine for a flat list but
    /// ambiguous once rows have to point at each other.
    fn folder(id: i64, parent: Option<i64>, path: &str) -> Mailbox {
        let mut m = mailbox(path, MailboxRole::Regular, counts(0, 0, 0));
        m.id = MailboxId::new(id);
        m.parent_id = parent.map(MailboxId::new);
        m
    }

    /// #324's own acceptance criterion: today's flat list cannot tell these
    /// apart at all. Nested and indented, they can.
    #[test]
    fn two_folders_sharing_a_leaf_name_under_different_parents_are_distinguishable() {
        let mailboxes = vec![
            folder(1, None, "Clients"),
            folder(2, Some(1), "Clients/Old"),
            folder(3, None, "Archive2024"),
            folder(4, Some(3), "Archive2024/Old"),
        ];
        let rows = folder_rows(&mailboxes, &HashSet::new());
        let old: Vec<&FolderRow> = rows.iter().filter(|r| r.mailbox.name == "Old").collect();
        assert_eq!(old.len(), 2, "both `Old` folders should render");
        assert_ne!(
            old[0].mailbox.id, old[1].mailbox.id,
            "distinguishable rows, not the same row twice"
        );
        // Each sits under its own parent, one level deeper.
        for row in &old {
            assert_eq!(row.depth, 1);
        }
    }

    #[test]
    fn children_render_under_their_parent_indented_and_in_order() {
        let mailboxes = vec![
            folder(1, None, "Clients"),
            folder(2, Some(1), "Clients/Acme"),
            folder(3, Some(1), "Clients/Beta"),
            folder(4, None, "Newsletters"),
        ];
        let rows = folder_rows(&mailboxes, &HashSet::new());
        let names: Vec<&str> = rows.iter().map(|r| r.mailbox.name.as_str()).collect();
        assert_eq!(
            names,
            ["Clients", "Acme", "Beta", "Newsletters"],
            "a parent's children come right after it, sorted, before the next root"
        );
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].depth, 1);
        assert_eq!(rows[3].depth, 0);
        assert!(rows[0].has_children);
        assert!(!rows[1].has_children);
    }

    #[test]
    fn a_collapsed_parent_hides_its_children_but_still_has_a_row() {
        let mailboxes = vec![
            folder(1, None, "Clients"),
            folder(2, Some(1), "Clients/Acme"),
        ];
        let collapsed = HashSet::from([MailboxId::new(1)]);
        let rows = folder_rows(&mailboxes, &collapsed);
        let names: Vec<&str> = rows.iter().map(|r| r.mailbox.name.as_str()).collect();
        assert_eq!(names, ["Clients"], "the child is hidden while collapsed");
        assert!(
            rows[0].has_children,
            "but the parent still knows it has one"
        );
    }

    /// The design's own answer, taken directly: "A `\Noselect` parent cannot
    /// be opened but can be toggled" — so it needs a row when it organizes
    /// children, even though `sections`' flat list would have dropped it.
    #[test]
    fn a_noselect_parent_with_children_gets_a_toggle_only_row() {
        let mut clients = folder(1, None, "Clients");
        clients.selectable = false;
        let mailboxes = vec![clients, folder(2, Some(1), "Clients/Acme")];
        let rows = folder_rows(&mailboxes, &HashSet::new());
        let names: Vec<&str> = rows.iter().map(|r| r.mailbox.name.as_str()).collect();
        assert_eq!(names, ["Clients", "Acme"]);
        assert!(!rows[0].mailbox.selectable, "still not openable");
        assert!(rows[0].has_children);
    }

    #[test]
    fn a_noselect_folder_with_no_children_is_dropped() {
        let mut empty = folder(1, None, "Ghost");
        empty.selectable = false;
        let rows = folder_rows(&[empty], &HashSet::new());
        assert!(rows.is_empty(), "nothing to open, nothing to toggle");
    }

    /// `postio-sync::discover::link_parents`'s own promise: an intermediate
    /// level the server never listed leaves the child's `parent_id` pointing
    /// at nothing this account knows about, and the folder "just sits at the
    /// top" rather than vanishing or panicking.
    #[test]
    fn a_hierarchy_missing_its_intermediate_level_still_renders() {
        let mut orphan = folder(2, Some(99), "Clients/Acme");
        orphan.parent_id = Some(MailboxId::new(99)); // never listed
        let rows = folder_rows(&[orphan], &HashSet::new());
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].depth, 0,
            "sits at the top, not nested under nothing"
        );
    }

    #[test]
    fn deep_nesting_stops_indenting_past_the_stated_depth() {
        let mut mailboxes = vec![folder(1, None, "L0")];
        for i in 1..8 {
            mailboxes.push(folder(i + 1, Some(i), &format!("L{i}")));
        }
        let rows = folder_rows(&mailboxes, &HashSet::new());
        assert_eq!(rows.len(), 8);
        let max_depth = rows.iter().map(|r| r.depth).max().unwrap();
        assert_eq!(
            max_depth, MAX_DEPTH,
            "indentation stops at a stated depth rather than growing forever"
        );
        // The last two rows are at different real depths but the same
        // rendered one.
        assert_eq!(rows[6].depth, rows[7].depth);
    }

    #[test]
    fn ancestors_are_named_nearest_first() {
        let mailboxes = vec![
            folder(1, None, "Clients"),
            folder(2, Some(1), "Clients/Acme"),
            folder(3, Some(2), "Clients/Acme/Invoices"),
        ];
        assert_eq!(
            ancestors_of(&mailboxes, MailboxId::new(3)),
            vec![MailboxId::new(2), MailboxId::new(1)]
        );
        assert_eq!(ancestors_of(&mailboxes, MailboxId::new(1)), Vec::new());
    }

    #[test]
    fn the_status_line_reads_like_the_canvas() {
        let now = Instant::now();
        let status = SyncStatus {
            state: ConnectionState::Online,
            last_sync: now.checked_sub(Duration::from_secs(12)),
            ..SyncStatus::default()
        };
        assert_eq!(
            status.lines(now),
            ("idle · imap".to_string(), "last sync 12s".to_string())
        );
    }

    #[test]
    fn every_connection_state_says_something_true() {
        let now = Instant::now();
        let at = |state| SyncStatus {
            state,
            last_sync: now.checked_sub(Duration::from_secs(90)),
            ..SyncStatus::default()
        };

        assert_eq!(at(ConnectionState::Offline).lines(now).0, "offline · imap");
        assert_eq!(
            at(ConnectionState::Connecting).lines(now).0,
            "connecting · imap"
        );
        assert_eq!(at(ConnectionState::Online).lines(now).0, "idle · imap");
        assert_eq!(
            at(ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth
            })
            .lines(now)
            .0,
            "error · imap"
        );

        // Offline still says when it last worked: that is the useful half.
        assert_eq!(at(ConnectionState::Offline).lines(now).1, "last sync 1m");
    }

    #[test]
    fn a_resync_shows_its_progress_and_then_stops() {
        let now = Instant::now();
        let syncing = |done, total| SyncStatus {
            state: ConnectionState::Online,
            progress: Some((done, total)),
            ..SyncStatus::default()
        };

        assert_eq!(syncing(0, 400).lines(now).0, "syncing · imap");
        assert_eq!(syncing(168, 400).lines(now).0, "syncing · imap");
        assert_eq!(
            syncing(400, 400).lines(now).0,
            "idle · imap",
            "a finished resync is not a resync"
        );
        assert_eq!(
            syncing(4, 0).lines(now).0,
            "idle · imap",
            "a pass with nothing to reach is not a pass in progress"
        );
    }

    /// Issue #74. The long phase of a first sync was reported as nothing
    /// happening, which is worse than saying nothing: a user watching `idle`
    /// while the log fetches bodies concludes it is stuck.
    #[test]
    fn a_backfill_in_flight_is_not_reported_as_idle() {
        let now = Instant::now();
        let filling = SyncStatus {
            state: ConnectionState::Online,
            backfill: Some((412, 2000)),
            last_sync: Some(now),
            ..SyncStatus::default()
        };

        let (state, detail) = filling.lines(now);

        assert_ne!(
            state, "idle · imap",
            "bodies are downloading and the line claims nothing is happening"
        );
        assert_eq!(state, "downloading · imap");
        assert_eq!(
            detail, "bodies 412 of 2000",
            "the count is what answers `is anything happening`"
        );
    }

    /// The list being incomplete matters more than the bodies being
    /// incomplete: a mailbox mid-initial-sync is not usable yet, and one
    /// whose bodies are still arriving is.
    #[test]
    fn an_initial_sync_outranks_a_backfill_on_the_status_line() {
        let now = Instant::now();
        let both = SyncStatus {
            state: ConnectionState::Online,
            progress: Some((1204, 60_000)),
            backfill: Some((412, 2000)),
            ..SyncStatus::default()
        };

        let (state, detail) = both.lines(now);

        assert_eq!(state, "syncing · imap");
        assert_eq!(detail, "fetched 1204");
    }

    /// A finished backfill is not a running one, and must fall back to the
    /// ordinary idle line rather than sticking at `2000 of 2000`.
    #[test]
    fn a_settled_backfill_stops_claiming_to_be_downloading() {
        let now = Instant::now();
        for backfill in [Some((2000, 2000)), Some((0, 0)), None] {
            let done = SyncStatus {
                state: ConnectionState::Online,
                backfill,
                last_sync: Some(now),
                ..SyncStatus::default()
            };
            let (state, _) = done.lines(now);
            assert_eq!(
                state, "idle · imap",
                "backfill {backfill:?} is not in flight and must read idle"
            );
        }
    }

    #[test]
    fn a_first_sync_in_progress_does_not_also_claim_it_never_synced() {
        // `postio-qhz.6`: the first live sync reported "0% synced" and "never
        // synced" at the same time. Both were true by their own measure —
        // progress came from `SyncProgress`, "never synced" from a
        // `last_synced_at` that stays unset until a pass *completes* — and
        // together they told the user nothing about whether anything was
        // happening.
        let now = Instant::now();
        let first_sync = SyncStatus {
            state: ConnectionState::Online,
            progress: Some((1204, 60_000)),
            last_sync: None,
            ..SyncStatus::default()
        };

        let (state, detail) = first_sync.lines(now);

        assert_eq!(state, "syncing · imap");
        assert_eq!(
            detail, "fetched 1204",
            "a pass that is running has to say so on both lines, not report \
             progress on one and deny it on the other"
        );
        assert!(
            !detail.contains("never"),
            "still claiming it never synced while it is syncing: {detail}"
        );
    }

    #[test]
    fn a_percentage_is_never_shown_against_an_upper_bound() {
        // The denominator is `UIDNEXT - 1` — the highest UID a pass *could*
        // reach, which expunged messages leave gaps in. A pass routinely
        // finishes well short of it, so a percentage of it is a number that
        // does not mean what it looks like. The count does.
        let now = Instant::now();
        let syncing = SyncStatus {
            state: ConnectionState::Online,
            progress: Some((9, 60_000)),
            ..SyncStatus::default()
        };

        let (state, detail) = syncing.lines(now);

        assert!(!state.contains('%'), "still a percentage: {state}");
        assert!(!detail.contains('%'), "still a percentage: {detail}");
        assert_eq!(detail, "fetched 9");
    }

    #[test]
    fn a_finished_pass_says_when_rather_than_how_many() {
        let now = Instant::now();
        let synced = SyncStatus {
            state: ConnectionState::Online,
            last_sync: now.checked_sub(Duration::from_secs(12)),
            progress: None,
            ..SyncStatus::default()
        };

        assert_eq!(
            synced.lines(now),
            ("idle · imap".into(), "last sync 12s".into())
        );
    }

    #[test]
    fn a_failure_still_wins_over_a_pass_that_was_running() {
        // The reason is what the user has to act on. A count of what had been
        // fetched before it broke is not.
        let now = Instant::now();
        let failing = SyncStatus {
            state: ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            },
            progress: Some((40, 400)),
            detail: Some("the password was refused".into()),
            ..SyncStatus::default()
        };

        assert_eq!(
            failing.lines(now),
            ("error · imap".into(), "the password was refused".into())
        );
    }

    #[test]
    fn an_error_says_why_instead_of_when() {
        let now = Instant::now();
        let status = SyncStatus {
            state: ConnectionState::Failing {
                reason: postio_core::FailureReason::Auth,
            },
            last_sync: now.checked_sub(Duration::from_secs(4 * 3600)),
            detail: Some("app-specific password rejected".to_string()),
            ..SyncStatus::default()
        };
        assert_eq!(
            status.lines(now),
            (
                "error · imap".to_string(),
                "app-specific password rejected".to_string()
            ),
            "`last sync 4h` is not what you need when the password expired"
        );
    }

    #[test]
    fn a_sync_that_never_happened_says_so() {
        let now = Instant::now();
        assert_eq!(SyncStatus::default().lines(now).1, "never synced");
        assert_eq!(SyncStatus::default().refresh_interval(now), None);
    }

    #[test]
    fn ages_are_compact_and_step_down_through_the_units() {
        assert_eq!(age(Duration::from_secs(0)), "0s");
        assert_eq!(age(Duration::from_secs(59)), "59s");
        assert_eq!(age(Duration::from_secs(60)), "1m");
        assert_eq!(age(Duration::from_secs(3599)), "59m");
        assert_eq!(age(Duration::from_secs(3600)), "1h");
        assert_eq!(age(Duration::from_secs(86_399)), "23h");
        assert_eq!(age(Duration::from_secs(86_400)), "1d");
        assert_eq!(age(Duration::from_secs(9 * 86_400)), "9d");
    }

    #[test]
    fn the_status_line_only_ticks_as_fast_as_it_changes() {
        let now = Instant::now();
        let synced = |ago| SyncStatus {
            state: ConnectionState::Online,
            last_sync: now.checked_sub(Duration::from_secs(ago)),
            ..SyncStatus::default()
        };

        assert_eq!(
            synced(12).refresh_interval(now),
            Some(Duration::from_secs(1)),
            "seconds matter while the answer is in seconds"
        );
        assert_eq!(
            synced(300).refresh_interval(now),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            synced(4 * 3600).refresh_interval(now),
            Some(Duration::from_secs(300)),
            "an age in hours must not wake the process every second"
        );
    }
}
