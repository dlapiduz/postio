import Foundation
import PostioFFI

/// The first-run wizard (canvas 09): an address, a password, what was found,
/// and `Connect`.
///
/// The desktop's first run in its three steps (`postio_ui::onboarding`):
/// naming the account and proving you own it are steps 1 and 2 of the one
/// form, and how far back to sync is step 3. The looking-up and the proof
/// are the boundary's -- the shared onboarding the desktop uses -- and this
/// holds only what the screen decides: when to look an address up, when
/// `Connect` may be pressed, and what a refusal does to the form.
///
/// The boundary arrives as three closures rather than a session, so those
/// decisions are tested without a network or a store.
@MainActor
@Observable
public final class FirstRunModel {
    /// Where the wizard is.
    public enum Phase: Equatable {
        /// The form: address, password, servers.
        case account
        /// Signing in to the server, before anything is written.
        case connecting
        /// The account is saved; how far back should the first sync reach?
        case syncWindow
        /// Done. The window becomes the mail it was waiting for.
        case finished
    }

    public typealias LookUp = @MainActor (String) async -> DiscoveredFfi
    public typealias Connect = @MainActor (NewAccountFfi) async -> String?
    public typealias WriteWindow = @MainActor (SyncWindowFfi) -> String?

    private let lookUpAddress: LookUp
    private let connectAccount: Connect
    private let writeWindow: WriteWindow

    /// What mail arrives at. Changing it takes away a card that was about
    /// another address: Connect would otherwise dial the old domain's servers.
    public var address = "" {
        didSet {
            guard address != oldValue else { return }
            problem = nil
            if login.isEmpty || login == oldValue { login = address }
            if discovered != nil, address != lookedUp {
                discovered = nil
                lookedUp = nil
                if !editing { clearServers() }
            }
        }
    }

    /// On its way to the keyring, and nowhere else.
    public var password = "" {
        didSet { if password != oldValue { problem = nil } }
    }

    /// The name mail is sent as. Optional: empty means the address.
    public var name = ""

    /// What looking the address up found, once it has been.
    public private(set) var discovered: DiscoveredFfi?
    /// Whether a lookup is out.
    public private(set) var lookingUp = false
    /// Whether the servers are an open form rather than a card. A guess
    /// opens it; so does `Edit manually`.
    public var editing = false

    public var imapHost = ""
    public var imapPort = 993
    public var imapSecurity: SecurityFfi = .tls
    public var smtpHost = ""
    public var smtpPort = 465
    public var smtpSecurity: SecurityFfi = .tls
    /// What to sign in as -- the address, unless the provider says otherwise.
    public var login = ""

    public private(set) var phase: Phase = .account
    /// What went wrong, in words for the person reading them.
    public private(set) var problem: String?

    /// Step 3's choices, and the one picked.
    public let choices: [SyncWindowChoiceFfi]
    public var window: SyncWindowFfi

    /// The address the card is about.
    private var lookedUp: String?

    public init(
        choices: [SyncWindowChoiceFfi],
        lookUp: @escaping LookUp,
        connect: @escaping Connect,
        writeWindow: @escaping WriteWindow
    ) {
        self.choices = choices
        self.window = choices.first(where: \.recommended)?.window ?? .lastYear
        self.lookUpAddress = lookUp
        self.connectAccount = connect
        self.writeWindow = writeWindow
    }

    /// Over a real session: each call leaves the main actor, because each
    /// waits on the network.
    public convenience init(session: PostioSession) {
        self.init(
            choices: syncWindowChoices(),
            lookUp: { address in
                await Task.detached { session.discoverAccount(address) }.value
            },
            connect: { account in
                await Task.detached { session.connectAccount(account) }.value
            },
            writeWindow: { writeInitialSyncWindow(window: $0) }
        )
    }

    /// `1 of 3`, the desktop's count (`postio_ui::onboarding::step_of`).
    public var counter: String {
        switch phase {
        case .account: "1 of 3"
        case .connecting: "2 of 3"
        case .syncWindow, .finished: "3 of 3"
        }
    }

    /// Whether the provider signs in through the browser rather than with a
    /// password. Postio never draws a provider's login form, so the wizard
    /// sends that person to the browser route instead of asking.
    public var signsInWithBrowser: Bool { discovered?.browserSignIn ?? false }

    /// Whether `Connect` does anything yet.
    ///
    /// Before the lookup it may be pressed -- `Connect` looks the address up
    /// first. After it, there must be servers to sign in to.
    public var canConnect: Bool {
        guard phase == .account, !lookingUp, !signsInWithBrowser,
              looksLikeAnAddress(address: address), !password.isEmpty
        else { return false }
        if discovered == nil { return true }
        return !imapHost.trimmingCharacters(in: .whitespaces).isEmpty
            && !smtpHost.trimmingCharacters(in: .whitespaces).isEmpty
    }

    /// Look the address up, if it is one and has not been already. Called
    /// when the field is left, never per keystroke: each lookup is DNS and
    /// HTTPS to the address's domain.
    public func lookUp() async {
        let asked = address.trimmingCharacters(in: .whitespaces)
        guard looksLikeAnAddress(address: asked), asked != lookedUp, !lookingUp else { return }
        lookingUp = true
        let card = await lookUpAddress(asked)
        lookingUp = false
        // Typed over while the lookup was out: the answer is about an
        // address nobody is asking about any more.
        guard address.trimmingCharacters(in: .whitespaces) == asked else { return }
        lookedUp = asked
        discovered = card
        imapHost = card.imap.host
        imapPort = Int(card.imap.port)
        imapSecurity = card.imap.security
        smtpHost = card.smtp.host
        smtpPort = Int(card.smtp.port)
        smtpSecurity = card.smtp.security
        login = card.login
        editing = !card.found
    }

    /// Sign in and, only if that works, save the account.
    public func connect() async {
        if discovered == nil { await lookUp() }
        guard canConnect else { return }
        problem = nil
        phase = .connecting
        let account = NewAccountFfi(
            address: address.trimmingCharacters(in: .whitespaces),
            name: name,
            password: password,
            login: login,
            imap: ServerFfi(host: imapHost, port: UInt16(clamping: imapPort), security: imapSecurity),
            smtp: ServerFfi(host: smtpHost, port: UInt16(clamping: smtpPort), security: smtpSecurity)
        )
        if let complaint = await connectAccount(account) {
            problem = complaint
            phase = .account
            return
        }
        // Saved. The keyring has the password now; this copy is not needed.
        password = ""
        phase = .syncWindow
    }

    /// Step 3's button. A failed write costs the size picked, not the
    /// account, so the wizard finishes either way and says so.
    public func startSync() {
        guard phase == .syncWindow else { return }
        problem = writeWindow(window)
        phase = .finished
    }

    private func clearServers() {
        imapHost = ""
        smtpHost = ""
        imapPort = 993
        smtpPort = 465
        imapSecurity = .tls
        smtpSecurity = .tls
    }
}
