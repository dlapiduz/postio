//! Focus at the boundary (specs/009-focus-macos, contracts/ffi-focus.md).
//!
//! The Mac app is Focus, and what Focus shows comes from the host's Focus
//! requests -- the same ones the GTK app and the terminal make through
//! `postio-client`. Each crosses here as a plain record.

use crate::session::{Session, SessionError, blocking};

/// What the header strip counts: the inbox's conversations, how many are
/// unread, how many need an action (the Has action toggle's number), and how
/// many messages were filed away today ("186 filtered today").
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct FocusCountsFfi {
    /// The conversations in Focus's inbox.
    pub conversations: u32,
    /// Of those, the ones with unread mail.
    pub unread: u32,
    /// Of those, the ones that draw a marker.
    pub has_action: u32,
    /// Messages filed away since local midnight.
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

/// The header strip's words, composed by the functions GTK's strip uses
/// (`postio_ui::focus_row`, `postio_ui::filtered`), under the same rules: the
/// filtered count only while filtering is on and something was filed today,
/// the digest rules only while there are some (spec 007 C10).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct FocusStripFfi {
    /// "312 · 41 unread".
    pub counts: String,
    /// "Has action · 7".
    pub has_action: String,
    /// "186 filtered today", or nothing.
    pub filtered_today: Option<String>,
    /// "4 digest rules", or nothing.
    pub digest_rules: Option<String>,
}

#[uniffi::export]
impl Session {
    /// Show one of Focus's lists: counted first, then `FocusListChanged`.
    pub fn open_focus(&self, scope: crate::FocusScopeFfi) {
        self.focus_driver().open(scope.into());
    }

    /// How many rows Focus's list draws.
    pub fn focus_row_count(&self) -> u32 {
        self.focus_driver().row_count()
    }

    /// The row at `position`, or `None` while its page is on its way
    /// (`FocusPageReady` says when). Synchronous and does no I/O: what the
    /// table calls for every visible row.
    pub fn focus_row_at(&self, position: u32) -> Option<crate::FocusRowFfi> {
        self.focus_driver().row_at(position)
    }

    /// The pointer put the cursor on `position`: a plain click on a row.
    /// The controller says where the cursor went, as `FocusCursor`.
    pub fn focus_point(&self, position: u32) {
        self.focus_driver()
            .input(postio_focus::Input::Point(position));
    }

    /// A modified click on `position`: `range` when Shift was held, a
    /// toggle when the platform's toggle modifier was (Command on the Mac).
    pub fn focus_pick(&self, position: u32, range: bool) {
        self.focus_driver()
            .input(postio_focus::Input::Pick { position, range });
    }

    /// Whether the list stands scrolled to its very top: where it goes back
    /// to when an undo brings rows in above (`FocusListToTop`).
    pub fn focus_at_top(&self, at_top: bool) {
        self.focus_driver()
            .input(postio_focus::Input::AtTop(at_top));
    }

    /// A surface opened over Focus's list: the keys are its now, and on the
    /// Mac it replaces the secondary window that was open (M4).
    pub fn focus_surface_opened(&self, kind: crate::SurfaceKindFfi) {
        self.focus_driver()
            .input(postio_focus::Input::SurfaceOpened(kind.into()));
    }

    /// A surface over Focus's list closed, however it was closed.
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

    /// What Undo would take back now, in the toast's words, or `None` when
    /// nothing can be: what the Edit menu names its Undo item with. Read,
    /// not taken; the engine's stack stays the only one (FR-041).
    pub fn undo_description(&self) -> Option<String> {
        let client = self.client()?;
        blocking(client.undo_top()).ok().flatten()
    }

    /// The header strip's words, read now.
    pub fn focus_strip(&self) -> Result<FocusStripFfi, SessionError> {
        let counts = self.focus_counts()?;
        let config = self.focus_config();
        let rules = config.digests.len();
        Ok(FocusStripFfi {
            counts: postio_ui::focus_row::strip_counts(counts.conversations, counts.unread),
            has_action: postio_ui::focus_row::has_action_label(Some(counts.has_action)),
            filtered_today: (config.filtering && counts.filtered_today > 0)
                .then(|| postio_ui::filtered::today(counts.filtered_today)),
            digest_rules: (rules > 0).then(|| postio_ui::focus_row::digest_rules(rules)),
        })
    }

    /// The header strip's counts, read now.
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
