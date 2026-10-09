//! The error an attachment's bytes come back with.
//!
//! `Session::part_bytes` is the one part call left at the boundary: the
//! reader asks for a named part's bytes, and only a named one -- **never
//! speculatively**. There is no "prefetch the attachments of the open
//! message" call here and there must not be one (`ARCHITECTURE.md` section
//! 11: did the user ask for it). The parts panel that listed, saved and
//! exported parts went with the three-pane shell; Focus's reader brings its
//! own attachments surface (specs/009-focus-macos).
//!
//! `part_bytes` is synchronous and may fetch, waiting up to thirty seconds
//! before it gives up, so it must never be called from the main actor.
//! Logs carry the part id, the byte count and the outcome -- never the
//! filename, which the sender chose and is as much the user's mail as the
//! body is.

/// Why a part could not be had, or could not be written.
///
/// One variant carrying the sentence, the shape [`crate::ComposeError`] uses
/// and for the same reason: the frontend's job is to show this to the person
/// who pressed save, not to branch on it. A save that quietly produced
/// nothing is the failure being avoided — a zero-byte file on disk looks like
/// a saved attachment and is not one.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PartsError {
    /// It could not be done, and this is why in words.
    #[error("{message}")]
    Refused {
        /// What went wrong.
        message: String,
    },
}

impl From<String> for PartsError {
    fn from(message: String) -> Self {
        PartsError::Refused { message }
    }
}
