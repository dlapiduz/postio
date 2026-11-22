import Foundation
import PostioFFI

/// The looking-up and the proof are the boundary's (shared onboarding with the desktop); this holds
/// when to look an address
/// up, when `Connect` may be pressed, and what a refusal does to the form. The boundary arrives as
/// three closures so those
/// decisions are tested without a network or store.
@MainActor
@Observable
public final class FirstRunModel {
    public enum Phase: Equatable {
        case account
        case connecting
        case syncWindow
        case finished
    }

    public typealias LookUp = @MainActor (String) async -> DiscoveredFfi
    public typealias Connect = @MainActor (NewAccountFfi) async -> String?
    public typealias WriteWindow = @MainActor (SyncWindowFfi) -> String?

    private let lookUpAddress: LookUp
    private let connectAccount: Connect
    private let writeWindow: WriteWindow

    /// Changing it takes away a card that was about another address: Connect would otherwise dial
    /// the old domain's servers.
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

    public var name = ""

    public private(set) var discovered: DiscoveredFfi?
    public private(set) var lookingUp = false
    public var editing = false

    public var imapHost = ""
    public var imapPort = 993
    public var imapSecurity: SecurityFfi = .tls
    public var smtpHost = ""
    public var smtpPort = 465
    public var smtpSecurity: SecurityFfi = .tls
    public var login = ""

    public private(set) var phase: Phase = .account
    public private(set) var problem: String?

    public let choices: [SyncWindowChoiceFfi]
    public var window: SyncWindowFfi

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

    public var counter: String {
        switch phase {
        case .account: "1 of 3"
        case .connecting: "2 of 3"
        case .syncWindow, .finished: "3 of 3"
        }
    }

    public var signsInWithBrowser: Bool { discovered?.browserSignIn ?? false }

    public var canConnect: Bool {
        guard phase == .account, !lookingUp, !signsInWithBrowser,
              looksLikeAnAddress(address: address), !password.isEmpty
        else { return false }
        if discovered == nil { return true }
        return !imapHost.trimmingCharacters(in: .whitespaces).isEmpty
            && !smtpHost.trimmingCharacters(in: .whitespaces).isEmpty
    }

    /// Called when the field is left, never per keystroke: each lookup is DNS and HTTPS to the
    /// address's domain.
    public func lookUp() async {
        let asked = address.trimmingCharacters(in: .whitespaces)
        guard looksLikeAnAddress(address: asked), asked != lookedUp, !lookingUp else { return }
        lookingUp = true
        let card = await lookUpAddress(asked)
        lookingUp = false
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
        password = ""
        phase = .syncWindow
    }

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
