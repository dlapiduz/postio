import Foundation
import PostioFFI

/// The three things the settings window can do to an account (#1277).
///
/// A model rather than three closures in a view, because two of them block —
/// a connection test waits on a server, a re-index walks every message — and
/// because the third is destructive and has to be *asked about* rather than
/// done. What a view keeps is which one is running and what it said.
@MainActor
@Observable
public final class AccountActions {
    /// Which action is running, if any, so a pane can disable the rest.
    public enum Running: Equatable {
        case testing, reindexing, removing
    }

    public private(set) var running: Running?
    /// What the last action said. Cleared when another starts.
    public private(set) var outcome: String?
    /// Whether that outcome was a failure, for how it is drawn.
    public private(set) var failed = false
    /// The account a removal is waiting to be confirmed for.
    public private(set) var confirmingRemoval: AccountFfi?

    public init() {}

    /// Whether anything is in flight.
    public var isBusy: Bool { running != nil }

    /// Test the connection, off the main actor.
    public func test(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil else { return }
        begin(.testing)
        let id = account.id
        let report = await Task.detached { session.testConnection(id) }.value
        finish(report.message, failed: !report.reachable)
    }

    /// Rebuild the index, off the main actor.
    public func reindex(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil else { return }
        begin(.reindexing)
        let id = account.id
        let complaint = await Task.detached { session.reindexAccount(id) }.value
        finish(
            complaint ?? "The search index was rebuilt from the mail already here.",
            failed: complaint != nil
        )
    }

    /// Ask before removing: it takes the account's mail with it.
    public func askToRemove(_ account: AccountFfi) {
        confirmingRemoval = account
    }

    /// Changed your mind.
    public func cancelRemoval() {
        confirmingRemoval = nil
    }

    /// What the confirmation says, naming what goes.
    ///
    /// The address, and the fact that the mail goes with it. A confirmation
    /// that says "are you sure?" and nothing else is a dialog people learn to
    /// dismiss without reading.
    public func removalWarning(for account: AccountFfi) -> String {
        """
        Remove \(account.address)? Its mail is removed from this Mac and its \
        password is taken out of your Keychain. Mail on the server is not \
        touched.
        """
    }

    /// Do it, having asked.
    public func confirmRemoval(through session: PostioSession?) async {
        guard let account = confirmingRemoval, let session, running == nil else { return }
        confirmingRemoval = nil
        begin(.removing)
        let id = account.id
        let complaint = await Task.detached { session.removeAccount(id) }.value
        finish(complaint ?? "\(account.address) was removed.", failed: complaint != nil)
    }

    private func begin(_ what: Running) {
        running = what
        outcome = nil
        failed = false
    }

    private func finish(_ said: String, failed: Bool) {
        running = nil
        outcome = said
        self.failed = failed
    }
}
