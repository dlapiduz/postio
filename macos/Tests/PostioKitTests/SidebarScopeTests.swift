import PostioFFI
import Testing

@testable import PostioKit

/// What picking a sidebar row asks the list for.
///
/// Most rows are folders and the answer is that folder. Three are not:
/// Flagged, Snoozed and the Outbox are **queries wearing a folder's
/// clothes** — `postio_ui::sidebar::view_rows` builds them with no path to
/// `SELECT` and no id, because there is no row in the store behind them.
///
/// Which is why this has to exist. `Engine.open(mailbox:)` asked for
/// `.mailbox(mailbox: row.id)` whatever the row was, and a view row's id is
/// `MailboxId::UNASSIGNED` — zero. So clicking Flagged opened *mailbox
/// zero*: no folder, no error, an empty list, and nothing anywhere saying
/// the click had been understood as nonsense.
///
/// `postio-gtk`'s `feed.rs::scope_of` is the same function on the other side,
/// down to the fallback.
@Suite struct SidebarScopeTests {
    private func row(_ role: MailboxRoleFfi, id: Int64 = 0, account: Int64 = 7) -> MailboxFfi {
        MailboxFfi(
            id: id,
            account: account,
            parent: nil,
            name: "",
            role: role,
            unread: 0,
            total: 3,
            selectable: true,
            lastSyncedAt: nil,
            special: true,
            flagged: 3,
            snoozed: 0
        )
    }

    @Test func aFolderRowOpensItsFolder() {
        #expect(SidebarScope.of(row(.inbox, id: 42)) == .mailbox(mailbox: 42))
        #expect(SidebarScope.of(row(.archive, id: 9)) == .mailbox(mailbox: 9))
    }

    @Test func aViewRowOpensItsQueryAndNotMailboxZero() {
        #expect(SidebarScope.of(row(.flagged)) == .flagged(account: 7))
        #expect(SidebarScope.of(row(.snoozed)) == .snoozed(account: 7))
    }

    @Test func aServerThatHasARealFlaggedFolderKeepsIt() {
        // `view_rows` skips the synthetic Flagged when the account already has
        // a `\Flagged` folder, and that one is a folder like any other — it
        // has an id and `SELECT` works on it.
        #expect(SidebarScope.of(row(.flagged, id: 31)) == .mailbox(mailbox: 31))
    }

    @Test func theOutboxIsAQueryTooAndNotAFolder() {
        #expect(SidebarScope.of(row(.outbox)) == .outbox(account: 7))
    }

    @Test func aViewWithNoAccountFallsBackToEverything() {
        // The row cannot narrow to an account it does not name. Unified is a
        // superset of what was asked for, which is the safe way to be wrong:
        // it never reads as an empty folder.
        #expect(SidebarScope.of(row(.flagged, account: 0)) == .unified)
    }
}
