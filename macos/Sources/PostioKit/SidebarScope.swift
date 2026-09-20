import PostioFFI

/// What picking a sidebar row asks the list for.
///
/// # The one place a row becomes a query
///
/// Most rows are folders and the answer is that folder. Three are not:
/// Flagged, Snoozed and the Outbox are **queries wearing a folder's
/// clothes**. `postio_ui::sidebar::view_rows` builds them with an empty path,
/// because there is nothing to `SELECT`, and with no id, because there is no
/// row in the store behind them.
///
/// `Engine.open(mailbox:)` asked for `.mailbox(mailbox: row.id)` whatever the
/// row was — and an unassigned `MailboxId` is zero. So clicking Flagged asked
/// for *mailbox zero*: no folder, no error, an empty list, and nothing
/// anywhere saying the click had been understood as nonsense. Which is the
/// worst shape a bug can take here, because an empty folder is a perfectly
/// ordinary thing to see.
///
/// `postio-gtk`'s `feed.rs::scope_of` is this function on the other side,
/// fallback included.
public enum SidebarScope {
    /// The scope `row` opens.
    public static func of(_ row: MailboxFfi) -> ScopeFfi {
        // An id means a real folder, whatever role it carries — a server that
        // publishes its own `\Flagged` folder gets a row with an id, and
        // `view_rows` skips the synthetic one for exactly that account.
        guard row.id == 0 else { return .mailbox(mailbox: row.id) }

        // A view with no account cannot narrow to one. Unified is a superset
        // of what was asked for, which is the safe way to be wrong: it never
        // reads as an empty folder.
        guard row.account != 0 else { return .unified }

        switch row.role {
        case .flagged: return .flagged(account: row.account)
        case .snoozed: return .snoozed(account: row.account)
        case .outbox: return .outbox(account: row.account)
        default:
            // No other role reaches here — `view_rows` builds exactly those
            // three and everything else has an id. The account is the one
            // safe answer: a superset, never a scope that silently matches
            // nothing.
            return .account(account: row.account)
        }
    }
}
