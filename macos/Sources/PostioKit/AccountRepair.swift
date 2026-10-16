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
/// What is wrong with an account's credential, when something is.
public enum CredentialTrouble: Equatable, Sendable {
    /// An OAuth grant past its expiry — the boundary's `needsAttention`.
    case expired
    /// The server refused the credential it was given: a rotated app
    /// password, or a revoked grant.
    case rejected
    /// Nothing in the Keychain to sign in with — the *Partial* state.
    case missing
}

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

    /// What is wrong with an account's credential, and what to press.
    public struct Offer: Equatable, Sendable {
        /// What is wrong, in words: a button with no reason beside it is a
        /// button nobody knows whether to press.
        public let reason: String
        /// What the button says, or `nil` when there is nothing this pane
        /// can put right — the row still says what is wrong.
        public let button: String?
    }

    /// What the row offers for `account`, or `nil` when its credential is
    /// fine as far as anything knows.
    ///
    /// `found` is what the last connection test said about this account
    /// (`AccountActions.trouble(for:)`); an expiry the boundary reports
    /// outranks it, because it is read fresh on every settings open.
    ///
    /// Every state a person can repair gets the same offer (#1584): an
    /// expired grant, a password the server refused, and none in the
    /// Keychain at all — which used to get a sentence and no way forward.
    /// The button is named after what it *does*, per route: "Reconnect" in
    /// front of a password field is a promise about a browser that is not
    /// going to open.
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
            // A local mail store, or a token a broker mints: nothing here
            // to sign in with, so nothing to press.
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

    /// Run when a repair has gone through, with the account it was for, so
    /// whatever was found wrong with it stops being said.
    public var repaired: ((Int64) -> Void)?

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
        if complaint == nil { repaired?(id) }
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
        if complaint == nil { repaired?(id) }
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
