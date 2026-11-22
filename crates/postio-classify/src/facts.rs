use postio_model::{EmailAddress, ThreadId};

/// Each answer is at most one seek or lookup: the filing pass pays for every one on every new
/// message.
///
/// **An implementation that cannot answer says `true`.** Every question is a reason not to act, so
/// a failed read keeps the mail in the inbox.
pub trait Facts {
    fn wrote_to(&self, address: &EmailAddress) -> bool;

    fn took_part(&self, thread: ThreadId) -> bool;

    fn own_domain(&self, address: &EmailAddress) -> bool;

    /// The user pinned `address`, or restored mail from it (`[focus.filter] never`).
    fn never_filter(&self, address: &EmailAddress) -> bool;
}
