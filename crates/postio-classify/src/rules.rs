//! The data the rules layer reads (`contracts/engine.md`, "The classifier").

use crate::digests::Digests;
use crate::senders::Senders;

/// Everything the classifier decides by, as data rather than code
/// (constitution VII: providers are data):
///
/// - the automated-senders table, shipped as TOML (T105);
/// - the user's digest rules, from `[[focus.digests]]` (T131, T133);
/// - the marker kinds the user stopped, from `[focus.filter] stop_markers`
///   (T118).
///
/// Each of those tasks adds what its layer reads here, so a rule can only
/// arrive as data the pipeline is handed.
///
/// The needs-action detector's own rules are not here. They are words, not
/// senders: plain code with a small compiled-in table, which is what FR-165
/// allows the built-in detector.
pub trait Rules {
    /// The automated-senders table: [`Senders::shipped`] in production. The
    /// needs-action question is not asked of mail from a sender in it
    /// (FR-106), and T122's filing pass files by it.
    fn senders(&self) -> &Senders;

    /// The user's digest rules (`[[focus.digests]]`, T133), in the file's
    /// order: the first that matches holds the message. None unless the
    /// rules say otherwise.
    fn digests(&self) -> &Digests {
        Digests::none()
    }
}
