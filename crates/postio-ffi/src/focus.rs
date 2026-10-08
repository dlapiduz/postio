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
