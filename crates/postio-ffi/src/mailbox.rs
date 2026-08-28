//! The folder tree, as the sidebar reads it.

use postio_model::mailbox::{Mailbox, MailboxRole};

/// What a folder is for.
///
/// Crosses because a sidebar built from names would be wrong in a way that
/// looks like a server bug: the role is what lets Postio say "Archive" rather
/// than `[Gmail]/All Mail`, and what makes `a` archive to the right place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MailboxRoleFfi {
    /// The account's inbox.
    Inbox,
    /// Where `a` / `A` archive to.
    Archive,
    /// Where sent mail is filed.
    Sent,
    /// Where unsent drafts live.
    Drafts,
    /// Where deleted mail is moved.
    Trash,
    /// Junk / spam.
    Junk,
    /// A flagged / starred view.
    Flagged,
    /// The snoozed view.
    Snoozed,
    /// An ordinary user folder.
    Regular,
}

impl From<MailboxRole> for MailboxRoleFfi {
    fn from(role: MailboxRole) -> Self {
        match role {
            MailboxRole::Inbox => MailboxRoleFfi::Inbox,
            MailboxRole::Archive => MailboxRoleFfi::Archive,
            MailboxRole::Sent => MailboxRoleFfi::Sent,
            MailboxRole::Drafts => MailboxRoleFfi::Drafts,
            MailboxRole::Trash => MailboxRoleFfi::Trash,
            MailboxRole::Junk => MailboxRoleFfi::Junk,
            MailboxRole::Flagged => MailboxRoleFfi::Flagged,
            MailboxRole::Snoozed => MailboxRoleFfi::Snoozed,
            MailboxRole::Regular => MailboxRoleFfi::Regular,
        }
    }
}

/// One folder, with what the sidebar draws beside it.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailboxFfi {
    /// Local id — what `openScope` takes.
    pub id: i64,
    /// The account it belongs to.
    pub account: i64,
    /// The parent folder, when there is one.
    ///
    /// Mailboxes are a tree, and flattening one turns a tidy account into a
    /// list of slash-separated strings with no way to recover the nesting.
    pub parent: Option<i64>,
    /// The leaf name, for display. Not the server path.
    pub name: String,
    /// What the folder is for.
    pub role: MailboxRoleFfi,
    /// Messages without `\Seen`. **Cached, not counted** — `MailboxCounts`
    /// exists so the sidebar never counts rows, and recounting here would undo
    /// that on the surface redrawn most often.
    pub unread: u32,
    /// Total messages.
    pub total: u32,
    /// Whether the folder can hold messages. A `\Noselect` folder is a
    /// container in the hierarchy and opening it shows nothing.
    pub selectable: bool,
}

impl From<Mailbox> for MailboxFfi {
    fn from(mailbox: Mailbox) -> Self {
        MailboxFfi {
            id: mailbox.id.into(),
            account: mailbox.account_id.into(),
            parent: mailbox.parent_id.map(Into::into),
            name: mailbox.name,
            role: mailbox.role.into(),
            unread: mailbox.counts.unread,
            total: mailbox.counts.total,
            selectable: mailbox.selectable,
        }
    }
}
