import PostioFFI

/// What makes two sidebar rows the same row.
///
/// # Why an id is not enough
///
/// Three rows in the sidebar are not folders. Flagged, Snoozed and the Outbox
/// are queries — `postio_ui::sidebar::view_rows` builds them with an empty
/// path and no id, because there is no row in the store behind them. So every
/// one of them carries `MailboxId::UNASSIGNED`, which is `0`, and so does
/// every one of them on the second account.
///
/// `ForEach(id: \.id)` and `List(selection:)` take that at its word. Two rows
/// claiming one identity is undefined behaviour in a list: selection lands on
/// whichever the diffing algorithm decided was "the" row, and which one that
/// is does not have to be stable between redraws.
///
/// The account and the role are what distinguish them, so the identity is all
/// three. For an ordinary folder the id alone still decides, because no two
/// folders share one.
public struct SidebarRowId: Hashable, Sendable {
    /// The account the row belongs to. `0` for a row that names none.
    public let account: Int64
    /// The folder, or `0` for a view.
    public let mailbox: Int64
    /// What the row is — the only thing that separates two views of one
    /// account.
    public let role: MailboxRoleFfi

    public init(account: Int64, mailbox: Int64, role: MailboxRoleFfi) {
        self.account = account
        self.mailbox = mailbox
        self.role = role
    }
}

extension MailboxFfi {
    /// This row's identity, for a `ForEach` or a `List` selection.
    public var rowId: SidebarRowId {
        SidebarRowId(account: account, mailbox: id, role: role)
    }

    /// Whether this row is a real folder rather than a query.
    ///
    /// A view has nothing to `SELECT`, nothing to sync, and no name of its
    /// own — several things want to know which kind of row they are holding.
    public var isView: Bool { id == 0 }
}
