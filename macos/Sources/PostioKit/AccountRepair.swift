import Observation
import PostioFFI

/// Every app password is eventually rotated and every OAuth grant expires. The route is the
/// boundary's
/// (`AccountFfi.repair`): a password asked for where a provider wants browser consent, or a browser
/// sent to fix an app password, is a dead end.
public enum CredentialTrouble: Equatable, Sendable {
    case expired
    case rejected
    case missing
}

@MainActor
@Observable
public final class AccountRepair {
    public private(set) var running: Int64?

    public private(set) var asking: Int64?
    public var typed = ""

    public private(set) var outcome: String?
    public private(set) var failed = false

    public init() {}

    public struct Offer: Equatable, Sendable {
        public let reason: String
        public let button: String?
    }

    /// An expiry the boundary reports outranks `found` (the last connection test), because it is
    /// read fresh on every settings open.
    /// The button is named after what it does per route: "Reconnect" in front of a password field
    /// promises a browser.
    public static func offer(
        for account: AccountFfi,
        found: CredentialTrouble?,
        running: Bool
    ) -> Offer? {
        guard let trouble = account.needsAttention ? .expired : found else { return nil }
        let held: String
        let button: String?
        switch account.repair {
        case .nothing:
            held = "credential"
            button = nil
        case .password:
            held = "password"
            button = running ? "Saving…" : "New password…"
        case .browser:
            held = "sign-in"
            button = running ? "Signing in…" : "Reconnect"
        }
        let reason = switch trouble {
        case .expired: "This account's \(held) has expired."
        case .rejected: "The server refused this account's \(held)."
        case .missing: "This account has no \(held) in your Keychain yet."
        }
        return Offer(reason: reason, button: button)
    }

    public var repaired: ((Int64) -> Void)?

    public func ask(_ account: AccountFfi) {
        asking = account.id
        typed = ""
        outcome = nil
        failed = false
    }

    public func cancel() {
        asking = nil
        typed = ""
    }

    /// The keyring blocks, and on a locked one until somebody unlocks it.
    public func save(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil, !typed.isEmpty else { return }
        let id = account.id
        let password = typed
        begin(id)
        let complaint = await Task.detached { session.repairCredential(id, password) }.value
        typed = ""
        asking = complaint == nil ? nil : asking
        finish(complaint ?? "The password was stored. Postio will try again.", failed: complaint != nil)
        if complaint == nil { repaired?(id) }
    }

    /// Returns when a person comes back from a browser tab, so never call it on the main actor.
    public func reconnect(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil else { return }
        let id = account.id
        begin(id)
        let complaint = await Task.detached { session.reconnectAccount(id) }.value
        finish(complaint ?? "\(account.address) is signed in again.", failed: complaint != nil)
        if complaint == nil { repaired?(id) }
    }

    public func begin(_ account: AccountFfi, through session: PostioSession?) async {
        switch account.repair {
        case .password: ask(account)
        case .browser: await reconnect(account, through: session)
        case .nothing: break
        }
    }

    private func begin(_ account: Int64) {
        running = account
        outcome = nil
        failed = false
    }

    private func finish(_ said: String, failed: Bool) {
        running = nil
        outcome = said
        self.failed = failed
    }
}
