import Foundation
import PostioFFI

@MainActor
@Observable
public final class AccountActions {
    public enum Running: Equatable {
        case testing, reindexing, removing
    }

    public private(set) var running: Running?
    public private(set) var outcome: String?
    public private(set) var failed = false
    public private(set) var confirmingRemoval: AccountFfi?
    /// Keyed by account because one model serves every row: a finding that followed the selection
    /// would tell the next
    /// account it has no password. Kept across other actions because it stays true until a repair
    /// says otherwise.
    private var finding: (account: Int64, trouble: CredentialTrouble)?

    public func trouble(for account: Int64) -> CredentialTrouble? {
        guard let finding, finding.account == account else { return nil }
        return finding.trouble
    }

    func found(_ report: ConnectionReportFfi, for account: Int64) {
        let trouble: CredentialTrouble? =
            report.missingCredential ? .missing
            : report.rejectedCredential ? .rejected
            : nil
        if let trouble {
            finding = (account, trouble)
        } else if finding?.account == account {
            finding = nil
        }
    }

    public func credentialRepaired(_ account: Int64) {
        if finding?.account == account { finding = nil }
    }

    public private(set) var progress: (done: UInt32, total: UInt32)?

    public var progressLabel: String? {
        guard let progress, progress.total > 0 else { return nil }
        return "\(Int(progress.done).formatted(.number)) of "
            + "\(Int(progress.total).formatted(.number))"
    }

    public func reindexProgressed(done: UInt32, total: UInt32) {
        guard running == .reindexing else { return }
        progress = (done, total)
    }

    /// The row alone changes nothing: engines start when the session opens and a new account has
    /// none (#1299).
    public var accountAdded: (() -> Void)?

    public func added() {
        accountAdded?()
    }

    public init() {}

    public var isBusy: Bool { running != nil }

    public func test(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil else { return }
        begin(.testing)
        let id = account.id
        let report = await Task.detached { session.testConnection(id) }.value
        found(report, for: id)
        finish(report.message, failed: !report.reachable)
    }

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

    public func askToRemove(_ account: AccountFfi) {
        confirmingRemoval = account
    }

    public func cancelRemoval() {
        confirmingRemoval = nil
    }

    public func removalWarning(for account: AccountFfi) -> String {
        """
        Remove \(account.address)? Its mail is removed from this Mac and its \
        password is taken out of your Keychain. Mail on the server is not \
        touched.
        """
    }

    public func confirmRemoval(through session: PostioSession?) async {
        guard let account = confirmingRemoval, let session, running == nil else { return }
        confirmingRemoval = nil
        begin(.removing)
        let id = account.id
        let complaint = await Task.detached { session.removeAccount(id) }.value
        finish(complaint ?? "\(account.address) was removed.", failed: complaint != nil)
    }

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

    /// The pane's switches write directly, but their refusals belong in the same place as every
    /// other outcome.
    public func said(_ outcome: String, failed: Bool) {
        self.outcome = outcome
        self.failed = failed
    }

    #if DEBUG
        func beginReindexForTesting() { begin(.reindexing) }
    #endif

    private func begin(_ what: Running) {
        running = what
        outcome = nil
        failed = false
        progress = nil
    }

    private func finish(_ said: String, failed: Bool) {
        running = nil
        progress = nil
        outcome = said
        self.failed = failed
    }
}
