use postio_model::mailbox::{Mailbox, MailboxRole};

/// What a folder is for: a sidebar built from names would show `[Gmail]/All Mail` for Archive, and
/// `a` would archive to the wrong place.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum MailboxRoleFfi {
    Inbox,
    Archive,
    Sent,
    Drafts,
    Trash,
    Junk,
    Flagged,
    Snoozed,
    Outbox,
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
            MailboxRole::Outbox => MailboxRoleFfi::Outbox,
            MailboxRole::Regular => MailboxRoleFfi::Regular,
        }
    }
}

impl From<MailboxRoleFfi> for MailboxRole {
    fn from(role: MailboxRoleFfi) -> Self {
        match role {
            MailboxRoleFfi::Inbox => MailboxRole::Inbox,
            MailboxRoleFfi::Archive => MailboxRole::Archive,
            MailboxRoleFfi::Sent => MailboxRole::Sent,
            MailboxRoleFfi::Drafts => MailboxRole::Drafts,
            MailboxRoleFfi::Trash => MailboxRole::Trash,
            MailboxRoleFfi::Junk => MailboxRole::Junk,
            MailboxRoleFfi::Flagged => MailboxRole::Flagged,
            MailboxRoleFfi::Snoozed => MailboxRole::Snoozed,
            MailboxRoleFfi::Outbox => MailboxRole::Outbox,
            MailboxRoleFfi::Regular => MailboxRole::Regular,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct MailboxFfi {
    /// Local id — what `openScope` takes.
    pub id: i64,
    pub account: i64,
    /// The parent folder, when there is one: mailboxes are a tree, not slash-separated strings.
    pub parent: Option<i64>,
    /// The leaf name, for display. Not the server path.
    pub name: String,
    pub role: MailboxRoleFfi,
    /// Messages without `\Seen`. Cached, not counted: recounting would undo `MailboxCounts` on the
    /// most-redrawn surface.
    pub unread: u32,
    pub total: u32,
    /// Whether the folder can hold messages. A `\Noselect` folder is a
    /// container in the hierarchy and opening it shows nothing.
    pub selectable: bool,
    /// When a sync pass over this folder last completed, in seconds; `None` for a folder that never
    /// finished one.
    pub last_synced_at: Option<i64>,
    /// Whether this row belongs in the sidebar's special-use section: not simply "has a role",
    /// since an account that has been through more than one client holds two folders per role and
    /// only the primary gets the treatment (#501).
    pub special: bool,
    /// How many messages carry `\Flagged`: the Flagged row's badge shows this rather than `unread`.
    pub flagged: u32,
    /// How many are snoozed, for the Snoozed row's badge; counted as away, so not a subset of
    /// `total`.
    pub snoozed: u32,
}

impl From<Mailbox> for MailboxFfi {
    /// `Session::mailboxes` decides `special`: a single mailbox cannot answer "am I the primary for
    /// my role".
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
            last_synced_at: mailbox.last_synced_at.map(|at| at.timestamp()),
            special: false,
            flagged: mailbox.counts.flagged,
            snoozed: mailbox.counts.snoozed,
        }
    }
}

/// What Postio calls a role, with no folder in hand; empty for an ordinary folder. For sentences
/// like "this account has no Drafts folder", where there is no folder to ask.
#[uniffi::export]
pub fn mailbox_role_name(role: MailboxRoleFfi) -> String {
    postio_ui::sidebar::role_name(role.into())
        .unwrap_or_default()
        .to_owned()
}
