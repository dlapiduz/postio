//! What the guards ask of the store and the config (`contracts/engine.md`,
//! "The classifier").

use postio_model::{EmailAddress, ThreadId};

/// The facts the guards stand on (FR-111). Each answer is at most one seek or
/// one lookup, because the filing pass pays for every one of them on every
/// new message.
///
/// **An implementation that cannot answer says `true`.** Every question here
/// is a reason *not* to act, so a read that fails keeps the mail where it is,
/// in the inbox: when in doubt, mail goes to the inbox (spec, US9).
pub trait Facts {
    /// The user has written to `address` (`correspondents`).
    fn wrote_to(&self, address: &EmailAddress) -> bool;

    /// The user took part in `thread`: a message of theirs is in it.
    fn took_part(&self, thread: ThreadId) -> bool;

    /// `address` is at one of the user's own domains, from their identities.
    fn own_domain(&self, address: &EmailAddress) -> bool;

    /// The user pinned `address`, or restored mail from it: `[focus.filter]
    /// never`.
    fn never_filter(&self, address: &EmailAddress) -> bool;
}
