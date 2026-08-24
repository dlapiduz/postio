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
use std::time::{Duration, Instant};

use adw::prelude::*;
use adw::subclass::prelude::*;
use gtk::glib;
use postio_core::ConnectionState;
use postio_model::ids::MailboxId;
use postio_model::mailbox::{Mailbox, MailboxRole};

/// What to call when the user picks a folder.
type SelectionHandler = Box<dyn Fn(MailboxId)>;

/// What to call when messages are dropped on a folder.
type DropHandler = Box<dyn Fn(crate::list_view::Dragged, MailboxId)>;

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
            ConnectionState::Failing => "error".to_string(),
            // A percentage only while there is a resync to be a percentage of.
            ConnectionState::Online => match self.progress {
                Some((done, total)) if total > 0 && done < total => {
                    format!("syncing {}%", (done as u64 * 100 / total as u64).min(99))
                }
                _ => "idle".to_string(),
            },
        }
    }

    /// The second line: the reason it is failing, or how long ago it worked.
    ///
    /// The reason wins. "last sync 4h" is not what someone needs to read when
    /// the password has expired.
    fn detail_line(&self, now: Instant) -> String {
        if self.state == ConnectionState::Failing
            && let Some(detail) = &self.detail
        {
            return detail.clone();
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
        match role_order(mailbox.role) {
            Some(_) => special.push(mailbox.clone()),
            None => ordinary.push(mailbox.clone()),
        }
    }

    special.sort_by_key(|m| (role_order(m.role).unwrap_or(u8::MAX), m.name.clone()));
    ordinary.sort_by_key(|m| m.path.to_lowercase());
    (special, ordinary)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct Sidebar {
        pub account: gtk::Label,
        pub special: gtk::ListBox,
        pub ordinary: gtk::ListBox,
        pub ordinary_section: gtk::Box,
        pub status_state: gtk::Label,
        pub status_detail: gtk::Label,
        pub status: RefCell<SyncStatus>,
        pub tick: RefCell<Option<glib::SourceId>>,
        pub selected: RefCell<Vec<SelectionHandler>>,
        pub dropped: RefCell<Vec<DropHandler>>,
        /// Set while a selection is being applied programmatically, so
        /// restoring one does not look like the user clicking it.
        pub echoing: std::cell::Cell<bool>,
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

        let scroller = gtk::ScrolledWindow::new();
        scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroller.set_vexpand(true);
        // Not a tab stop: the lists inside already move with the keyboard, so
        // stopping here would be a stop that does nothing and says nothing.
        scroller.set_focusable(false);
        scroller.set_child(Some(&folders));

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

        // Selecting in one list clears the other: together they are one list
        // of folders that happens to be drawn in two blocks.
        for (list, other) in [(&imp.special, &imp.ordinary), (&imp.ordinary, &imp.special)] {
            list.connect_row_selected(glib::clone!(
                #[weak(rename_to = sidebar)]
                self,
                #[weak]
                other,
                move |_, row| {
                    let Some(row) = row else { return };
                    other.unselect_all();
                    if sidebar.imp().echoing.get() {
                        return;
                    }
                    let id = MailboxId::new(unsafe { row_id(row) });
                    for callback in sidebar.imp().selected.borrow().iter() {
                        callback(id);
                    }
                }
            ));
        }

        self.set_child(Some(&column));
        self.set_status(SyncStatus::default());
    }

    /// The account address shown at the top.
    pub fn set_account(&self, address: &str) {
        self.imp().account.set_text(address);
    }

    /// Replace the folder list.
    ///
    /// Rows for mailboxes that are still there are updated in place rather
    /// than rebuilt, so a count changing does not take the selection or the
    /// keyboard focus with it — which is the whole of "counts update live".
    pub fn set_mailboxes(&self, mailboxes: &[Mailbox]) {
        let imp = self.imp();
        let (special, ordinary) = sections(mailboxes);
        let selected = self.selected();

        sync_rows(&imp.special, &special, self);
        sync_rows(&imp.ordinary, &ordinary, self);
        imp.ordinary_section.set_visible(!ordinary.is_empty());

        if let Some(id) = selected {
            self.select(id);
        }
    }

    /// The selected folder, if any.
    pub fn selected(&self) -> Option<MailboxId> {
        let imp = self.imp();
        for list in [&imp.special, &imp.ordinary] {
            if let Some(row) = list.selected_row() {
                return Some(MailboxId::new(unsafe { row_id(&row) }));
            }
        }
        None
    }

    /// Select a folder without reporting it back as a user action.
    pub fn select(&self, id: MailboxId) {
        let imp = self.imp();
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
            .and_then(|id| {
                rows.iter()
                    .find(|row| MailboxId::new(unsafe { row_id(row) }) == id)
            })
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
                .position(|row| MailboxId::new(unsafe { row_id(row) }) == id)
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
        let id = MailboxId::new(unsafe { row_id(row) });
        self.select(id);
        // `select` is deliberately quiet — it is what the window calls to
        // echo a folder it opened — so the keyboard has to announce its own
        // move, the same way a click does.
        for handler in self.imp().selected.borrow().iter() {
            handler(id);
        }
        Some(id)
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
            status.state == ConnectionState::Failing,
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
            let mailbox = MailboxId::new(unsafe { row_id(&row) });
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
            let mailbox = MailboxId::new(unsafe { row_id(&row) });
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
    unsafe { row.set_data("postio-mailbox-id", mailbox.id.get()) };

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

    name.set_text(&display_name(mailbox));
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
    row.update_property(&[gtk::accessible::Property::Label(&announce(mailbox))]);
}

/// What a screen reader says for a folder row.
fn announce(mailbox: &Mailbox) -> String {
    let name = display_name(mailbox);
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
pub fn display_name(mailbox: &Mailbox) -> String {
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

fn find_row(list: &gtk::ListBox, id: MailboxId) -> Option<gtk::ListBoxRow> {
    let mut index = 0;
    while let Some(row) = list.row_at_index(index) {
        if unsafe { row_id(&row) } == id.get() {
            return Some(row);
        }
        index += 1;
    }
    None
}

/// # Safety
///
/// The key is private to this module and only ever set by [`update_row`].
unsafe fn row_id(row: &gtk::ListBoxRow) -> i64 {
    unsafe {
        row.data::<i64>("postio-mailbox-id")
            .map(|p| *p.as_ref())
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
        let names: Vec<String> = special.iter().map(display_name).collect();
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
    fn a_folder_you_cannot_open_is_not_listed() {
        let mut container = mailbox("Lists", MailboxRole::Regular, counts(0, 0, 0));
        container.selectable = false;
        let (special, ordinary) = sections(&[container]);
        assert!(special.is_empty() && ordinary.is_empty());
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
        assert_eq!(at(ConnectionState::Failing).lines(now).0, "error · imap");

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

        assert_eq!(syncing(0, 400).lines(now).0, "syncing 0% · imap");
        assert_eq!(syncing(168, 400).lines(now).0, "syncing 42% · imap");
        assert_eq!(
            syncing(400, 400).lines(now).0,
            "idle · imap",
            "a finished resync is not a resync"
        );
        assert_eq!(
            syncing(4, 0).lines(now).0,
            "idle · imap",
            "a total of zero is not a percentage"
        );
    }

    #[test]
    fn an_error_says_why_instead_of_when() {
        let now = Instant::now();
        let status = SyncStatus {
            state: ConnectionState::Failing,
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
