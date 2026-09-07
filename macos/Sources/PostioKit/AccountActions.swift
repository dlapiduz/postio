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
    /// Whether the last test found no credential at all — the pane's
    /// *Partial* state, which asks for a password rather than a retry.
    public private(set) var missingCredential = false

    /// How far a re-index has got: messages done, and how many there are.
    ///
    /// A pass over five thousand messages takes long enough that a button
    /// with no progress is indistinguishable from a button that does nothing.
    public private(set) var progress: (done: UInt32, total: UInt32)?

    /// What the button says while it works — `Re-indexing… 1,203 of 4,985`.
    public var progressLabel: String? {
        guard let progress, progress.total > 0 else { return nil }
        return "\(Int(progress.done).formatted(.number)) of "
            + "\(Int(progress.total).formatted(.number))"
    }

    /// A report from the boundary. Ignored unless a re-index is running, so
    /// a late event cannot make an idle pane look busy.
    public func reindexProgressed(done: UInt32, total: UInt32) {
        guard running == .reindexing else { return }
        progress = (done, total)
    }

    /// What to run when an account has just been added.
    ///
    /// Set by the application, which owns the session and the accounts list.
    /// The sheet writes the row through the boundary and then has to say so:
    /// the row alone changes nothing, because the engines were started when
    /// the session opened and a new account has none (#1299).
    public var accountAdded: (() -> Void)?

    /// An account was added. Tell whoever is listening.
    public func added() {
        accountAdded?()
    }

    public init() {}

    /// Whether anything is in flight.
    public var isBusy: Bool { running != nil }

    /// Test the connection, off the main actor.
    public func test(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil else { return }
        begin(.testing)
        let id = account.id
        let report = await Task.detached { session.testConnection(id) }.value
        missingCredential = report.missingCredential
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

    /// Rename an account. Synchronous: it is one row, and no server is
    /// consulted.
    public func rename(
        _ account: AccountFfi,
        to name: String,
        through session: PostioSession?
    ) {
        guard let session, running == nil else { return }
        let complaint = session.setDisplayName(account.id, to: name)
        outcome = complaint ?? "Renamed to \(name.trimmingCharacters(in: .whitespaces))."
        failed = complaint != nil
    }

    /// Pretend a re-index started, so the reporting can be asserted without
    /// a session and a store behind it.
    #if DEBUG
        func beginReindexForTesting() { begin(.reindexing) }
    #endif

    private func begin(_ what: Running) {
        running = what
        outcome = nil
        failed = false
        missingCredential = false
        progress = nil
    }

    private func finish(_ said: String, failed: Bool) {
        running = nil
        progress = nil
        outcome = said
        self.failed = failed
    }
}
