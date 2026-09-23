import Observation
import PostioFFI

/// Putting a broken account back in service, and which of the two ways it
/// takes (#1584).
///
/// Every app password is eventually rotated and every OAuth grant eventually
/// expires, so this is the ordinary end of every account rather than an edge
/// case. **The route is the boundary's** — `AccountFfi.repair` — and that
/// matters: asking for a password where a provider wants browser consent is
/// asking for something no provider would accept, and sending somebody to a
/// browser to fix an app password sends them somewhere with no field to type
/// it in.
///
/// What is local is the running, the field, and what the button says while
/// it works.
@MainActor
@Observable
public final class AccountRepair {
    /// The account being repaired, if one is.
    public private(set) var running: Int64?

    /// The account a password is being asked for, and what has been typed.
    public private(set) var asking: Int64?
    public var typed = ""

    /// What the last attempt said, or `nil` before there has been one.
    public private(set) var outcome: String?
    /// Whether that outcome was a refusal, for how it is drawn.
    public private(set) var failed = false

    public init() {}

    /// What the button beside `account` says.
    ///
    /// Named after what it *does*, per route, rather than one word for both:
    /// "Reconnect" in front of a password field is a promise about a browser
    /// that is not going to open.
    public static func label(for account: AccountFfi, running: Bool) -> String? {
        guard account.needsAttention else { return nil }
        switch account.repair {
        case .nothing:
            // A local mail store signs in to nothing, so there is nothing to
            // press. The row still warns; it simply has no button.
            return nil
        case .password:
            return running ? "Saving…" : "New password…"
        case .browser:
            return running ? "Signing in…" : "Reconnect"
        }
    }

    /// Ask for a password for `account`.
    public func ask(_ account: AccountFfi) {
        asking = account.id
        typed = ""
        outcome = nil
        failed = false
    }

    /// Changed your mind.
    public func cancel() {
        asking = nil
        typed = ""
    }

    /// Store the password that was typed, off the main actor.
    ///
    /// The keyring blocks, and on a locked one it blocks until somebody
    /// unlocks it.
    public func save(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil, !typed.isEmpty else { return }
        let id = account.id
        let password = typed
        begin(id)
        let complaint = await Task.detached { session.repairCredential(id, password) }.value
        typed = ""
        asking = complaint == nil ? nil : asking
        finish(complaint ?? "The password was stored. Postio will try again.", failed: complaint != nil)
    }

    /// Sign in again through the system browser, off the main actor.
    ///
    /// Returns when the flow is over, which is when a person comes back from
    /// a browser tab — so this is not a call to make on the main actor at
    /// any price.
    public func reconnect(_ account: AccountFfi, through session: PostioSession?) async {
        guard let session, running == nil else { return }
        let id = account.id
        begin(id)
        let complaint = await Task.detached { session.reconnectAccount(id) }.value
        finish(complaint ?? "\(account.address) is signed in again.", failed: complaint != nil)
    }

    /// Start whichever repair `account` calls for.
    ///
    /// One entry point, so the button and any future command cannot take
    /// different routes for the same row.
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
