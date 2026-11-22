//! The error an attachment's bytes come back with. `Session::part_bytes` fetches only a named part,
//! never speculatively
//! (`ARCHITECTURE.md` §11); it is synchronous and may wait thirty seconds, so never call it from
//! the main actor.
//! Logs carry the part id, byte count and outcome, never the filename.

/// One variant carrying the sentence: the frontend shows it to the person who pressed save, not
/// branches on it.
#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum PartsError {
    #[error("{message}")]
    Refused { message: String },
}

impl From<String> for PartsError {
    fn from(message: String) -> Self {
        PartsError::Refused { message }
    }
}
