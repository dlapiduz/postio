//! Which messages a list is showing.
//!
//! Every reader of the message list — the store that pages through it, the
//! runtime that hands rows to a frontend, a frontend's own feed, and the FFI
//! boundary a second frontend crosses — answers the same question about the
//! same value, so it gets one type rather than a spelling per reader (#670).
//! `docs/engineering-notes.md`'s "Six types are called *Scope*" entry has the
//! full map of what does and does not belong here — in particular
//! [`crate::AccountScope`] answers a different question ("which accounts?")
//! and `postio_core::state::ViewScope` is deliberately *not* this type: it is
//! the narrower result of a rule applied to one.

use crate::ids::{AccountId, MailboxId, ThreadId};

/// Which messages a list shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListScope {
    /// One folder, as the server has it.
    Mailbox(MailboxId),
    /// Every folder in an account: the unified view.
    Account(AccountId),
    /// Everything flagged in an account, wherever it is filed.
    Flagged(AccountId),
    /// Everything currently snoozed in an account, wherever it is filed.
    Snoozed(AccountId),
    /// One conversation, wherever its messages are filed.
    ///
    /// Not a narrowing of a mailbox: a thread routinely spans folders, and a
    /// drill-in that filtered the list's own resident rows used to show only
    /// the part of it that happened to be paged in.
    Thread(ThreadId),
}

impl ListScope {
    /// The folder this scope names, when it names one.
    ///
    /// `None` for a smart folder or a thread — load-bearing wherever the
    /// caller goes on to use the answer as somewhere a message could be put.
    pub fn mailbox(self) -> Option<MailboxId> {
        match self {
            ListScope::Mailbox(id) => Some(id),
            ListScope::Account(_)
            | ListScope::Flagged(_)
            | ListScope::Snoozed(_)
            | ListScope::Thread(_) => None,
        }
    }
}
