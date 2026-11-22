//! Focus at the boundary: the host's Focus requests, each crossing as a plain record.

use crate::session::{Session, SessionError, blocking};

#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FocusCountsFfi {
    pub conversations: u32,
    pub unread: u32,
    pub has_action: u32,
    pub filtered_today: u32,
}

impl From<postio_client::protocol::FocusCounts> for FocusCountsFfi {
    fn from(counts: postio_client::protocol::FocusCounts) -> Self {
        FocusCountsFfi {
            conversations: counts.conversations,
            unread: counts.unread,
            has_action: counts.has_action,
            filtered_today: counts.filtered_today,
        }
    }
}

/// The header strip's words, composed by the functions GTK's strip uses (`postio_ui::focus_row`,
/// `postio_ui::filtered`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusStripFfi {
    /// "312 · 41 unread".
    pub counts: String,
    /// The unread figure as a number, for the Dock badge; it rides the strip's own read.
    pub unread: u32,
    /// "Has action · 7".
    pub has_action: String,
    /// "186 filtered today", or nothing.
    pub filtered_today: Option<String>,
    /// "4 digest rules", or nothing.
    pub digest_rules: Option<String>,
}

/// What `[sync]` says about telling somebody mail is waiting; `UiEvent::NoticesChanged` says when
/// it moved.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NoticesFfi {
    /// `[sync] notify`: post a notification for new mail.
    pub notify: bool,
    /// `[sync] badge`: show the unread count on the app's icon.
    pub badge: bool,
    /// `[sync] notify_roles`: the roles of the folders whose arrivals notify.
    pub notify_roles: Vec<String>,
}

impl From<&postio_config::SyncConfig> for NoticesFfi {
    fn from(sync: &postio_config::SyncConfig) -> Self {
        NoticesFfi {
            notify: sync.notify,
            badge: sync.badge,
            notify_roles: sync.notify_roles.clone(),
        }
    }
}

#[uniffi::export]
impl Session {
    pub fn open_focus(&self, scope: crate::FocusScopeFfi) {
        self.focus_driver().open(scope.into());
    }

    pub fn focus_row_count(&self) -> u32 {
        self.focus_driver().row_count()
    }

    /// The row at `position`, or `None` while its page is on its way (`FocusPageReady`).
    /// Synchronous, no I/O.
    pub fn focus_row_at(&self, position: u32) -> Option<crate::FocusRowFfi> {
        self.focus_driver().row_at(position)
    }

    /// The row if its page is already resident, else `None`; reads nothing, so the table can
    /// re-measure without paging.
    pub fn focus_resident_row_at(&self, position: u32) -> Option<crate::FocusRowFfi> {
        self.focus_driver().row_resident(position)
    }

    /// A plain click on a row; the controller says where the cursor went, as `FocusCursor`.
    pub fn focus_point(&self, position: u32) {
        self.focus_driver()
            .input(postio_focus::Input::Point(position));
    }

    /// A click on a thread row's chevron: the cursor goes there and the thread opens in place, or
    /// folds when open.
    pub fn focus_chevron_clicked(&self, position: u32) {
        self.focus_driver().chevron_clicked(position);
    }

    /// A double-click: the cursor goes to the row and its message opens, as Return does.
    pub fn focus_activate(&self, position: u32) {
        let driver = self.focus_driver();
        driver.input(postio_focus::Input::Point(position));
        driver.command(postio_core::CommandId::OpenMessage);
    }

    /// A modified click on `position`: `range` when Shift was held, a
    /// toggle when the platform's toggle modifier was (Command on the Mac).
    pub fn focus_pick(&self, position: u32, range: bool) {
        self.focus_driver()
            .input(postio_focus::Input::Pick { position, range });
    }

    /// Whether the list is scrolled to its very top, where it returns when an undo brings rows in
    /// above.
    pub fn focus_at_top(&self, at_top: bool) {
        self.focus_driver()
            .input(postio_focus::Input::AtTop(at_top));
    }

    /// A surface opened over the list: the keys are its now, and on the Mac it replaces the
    /// secondary window open.
    pub fn focus_surface_opened(&self, kind: crate::SurfaceKindFfi) {
        self.focus_driver()
            .input(postio_focus::Input::SurfaceOpened(kind.into()));
    }

    pub fn focus_surface_closed(&self, kind: crate::SurfaceKindFfi) {
        self.focus_driver()
            .input(postio_focus::Input::SurfaceClosed(kind.into()));
    }

    /// The open message's More menu and find, as they are now: what Back
    /// closes first.
    pub fn focus_reader_state(&self, more_open: bool, finding: bool) {
        self.focus_driver()
            .input(postio_focus::Input::ReaderState { more_open, finding });
    }

    /// The message window's width allows `chips` thread-bar chips: sent
    /// when it opens and on resize. The controller composes the bar again.
    pub fn focus_bar_room(&self, chips: u32) {
        self.focus_driver()
            .input(postio_focus::Input::BarRoom(chips));
    }

    /// A click on the thread bar's chip for member `index` (from 0): that
    /// message is shown, as `]` would show it.
    pub fn focus_thread_bar_chip_clicked(&self, index: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ThreadBarChipClicked(index));
    }

    /// A click on the bar's "... N more" chip, which stands for the members
    /// from `from` on: the outline opens on the first of them.
    pub fn focus_thread_bar_more_clicked(&self, from: u32) {
        self.focus_driver()
            .input(postio_focus::Input::ThreadBarMoreClicked { from });
    }

    /// The message window opened from a search has no more find matches in this direction: the
    /// controller goes on to the next matched message, or says it is the last.
    pub fn focus_match_edge(&self, forward: bool) {
        self.focus_driver()
            .input(postio_focus::Input::MatchEdge { forward });
    }

    pub fn focus_thread_menu_pick(&self, command: String) {
        if let Ok(id) = command.parse::<postio_core::CommandId>() {
            self.focus_driver()
                .input(postio_focus::Input::ThreadMenuPick(id));
        }
    }

    pub fn focus_outline_typed(&self, text: String) {
        self.focus_driver().input(postio_focus::Input::Outline(
            postio_focus::OutlineInput::Typed(text),
        ));
    }

    pub fn focus_outline_person(&self, name: Option<String>) {
        self.focus_driver().input(postio_focus::Input::Outline(
            postio_focus::OutlineInput::Person(name),
        ));
    }

    pub fn focus_outline_move(&self, by: i32) {
        self.focus_driver().input(postio_focus::Input::Outline(
            postio_focus::OutlineInput::Move(by),
        ));
    }

    /// Return, or a click on a row: show it. `None` is the cursor's row.
    pub fn focus_outline_show(&self, row: Option<u32>) {
        self.focus_driver().input(postio_focus::Input::Outline(
            postio_focus::OutlineInput::Show(row.map(|row| row as usize)),
        ));
    }

    pub fn focus_outline_focus_filter(&self) {
        self.focus_driver().input(postio_focus::Input::Outline(
            postio_focus::OutlineInput::FocusFilter,
        ));
    }

    /// What Undo would take back now, in the toast's words, or `None`. Read, not taken: the
    /// engine's stack stays the only one.
    pub fn undo_description(&self) -> Option<String> {
        let client = self.client()?;
        blocking(client.undo_top()).ok().flatten()
    }

    pub fn focus_strip(&self) -> Result<FocusStripFfi, SessionError> {
        let counts = self.focus_counts()?;
        let config = self.focus_config();
        let rules = config.digests.len();
        Ok(FocusStripFfi {
            counts: postio_ui::focus_row::strip_counts(counts.conversations, counts.unread),
            unread: counts.unread,
            has_action: postio_ui::focus_row::has_action_label(Some(counts.has_action)),
            filtered_today: (config.filtering && counts.filtered_today > 0)
                .then(|| postio_ui::filtered::today(counts.filtered_today)),
            digest_rules: (rules > 0).then(|| postio_ui::focus_row::digest_rules(rules)),
        })
    }

    /// Whether new mail notifies and whether the icon carries the unread
    /// count, as `config.toml` says now. No I/O: the session keeps it.
    pub fn notices(&self) -> NoticesFfi {
        self.notices_now()
    }

    pub fn focus_counts(&self) -> Result<FocusCountsFfi, SessionError> {
        let client = self
            .client()
            .ok_or_else(|| SessionError::StoreUnavailable {
                message: "The store is closed.".to_owned(),
            })?;
        blocking(client.focus_counts())
            .map(Into::into)
            .map_err(|error| SessionError::StoreUnavailable {
                message: error.to_string(),
            })
    }
}
