import Foundation
import PostioFFI

@MainActor
@Observable
public final class AddAccountModel: Identifiable {
    public let id = UUID()

    public enum Step: Int, CaseIterable {
        case address = 1
        case credentials = 2
        case store = 3
    }

    /// One case, and the type stays because a local store has a format that `LocalStore` states
    /// rather than guesses.
    /// A picker offering formats the application cannot open reads as a choice and costs a whole
    /// flow to find out.
    public enum Format: String, CaseIterable, Identifiable {
        case maildir
        public var id: String { rawValue }
    }

    public private(set) var step: Step = .address

    public var address = "" {
        didSet { refreshHint() }
    }
    public private(set) var hint: ProviderHintFfi?
    public var route: RouteFfi = .imap

    public var password = ""
    public var clientId = ""
    public var clientSecret = ""
    public private(set) var signingIn = false
    public private(set) var signInMessage = ""
    public var imapHost = ""
    public var imapPort = 993
    public var smtpHost = ""
    public var smtpPort = 465

    public var storePath: String
    public var format: Format = .maildir
    public var syncDays = 90

    public private(set) var problem: String?

    public init(storePath: String = "~/mail") {
        self.storePath = storePath
    }

    public var counter: String {
        let steps = steps
        let at = (steps.firstIndex(of: step) ?? 0) + 1
        return "\(at) of \(steps.count)"
    }

    private var steps: [Step] {
        route == .localStore ? [.address, .store] : Step.allCases
    }

    public var verdict: String {
        hint?.verdict ?? "Type an email address to begin."
    }

    /// Shallow on purpose: the server is the authority on whether an address exists.
    public var canContinue: Bool {
        switch step {
        case .address: address.contains("@") && !address.hasSuffix("@")
        case .credentials:
            switch route {
            case .imap: !password.isEmpty && !imapHost.isEmpty && !smtpHost.isEmpty
            case .outlook, .gmail: !clientId.trimmingCharacters(in: .whitespaces).isEmpty
            case .localStore: true
            }
        case .store: !storePath.isEmpty
        }
    }

    /// Read as the person types: a directory picked by mistake must say so where the field is, not
    /// after an account has been made that looks empty.
    public func storeProblem(through session: PostioSession?) -> String? {
        guard route == .localStore, !storePath.isEmpty else { return nil }
        return session?.inspectLocalStore(storePath)
    }

    public var estimate: String {
        switch syncDays {
        case 0: "everything on the server · however long that takes"
        case ...30: "about a month · minutes"
        case ...90: "about three months · minutes"
        case ...365: "about a year · tens of minutes"
        default: "several years · possibly hours"
        }
    }

    public func next() {
        guard canContinue else { return }
        problem = nil
        switch step {
        case .address:
            // The route is whatever is selected; putting the suggestion back here discarded the
            // person's choice.
            prefill()
            step = route == .localStore ? .store : .credentials
        case .credentials: step = .store
        case .store: break
        }
    }

    public func back() {
        problem = nil
        let steps = steps
        if let at = steps.firstIndex(of: step), at > 0 {
            step = steps[at - 1]
        } else {
            step = .address
        }
    }

    public private(set) var finishing = false

    /// The password route signs in first and writes only if that works: a wrong password otherwise
    /// became an account that failed every sync.
    public func finish(through session: PostioSession?) async -> Bool {
        switch route {
        case .imap:
            guard let session, !finishing else { return false }
            let account = NewAccountFfi(
                address: address,
                name: "",
                password: password,
                login: address,
                imap: ServerFfi(host: imapHost, port: UInt16(clamping: imapPort), security: .tls),
                smtp: ServerFfi(host: smtpHost, port: UInt16(clamping: smtpPort), security: .tls)
            )
            finishing = true
            let complaint = await Task.detached { session.connectAccount(account) }.value
            finishing = false
            if let complaint {
                problem = complaint
                return false
            }
            return true
        case .outlook, .gmail:
            problem = "Signing in happens in your browser — press Sign in."
            return false
        case .localStore:
            guard let session else { return false }
            if let complaint = session.addLocalAccount(address: address, path: storePath) {
                problem = complaint
                return false
            }
            return true
        }
    }

    /// Off the main actor: the flow blocks until the person comes back from Safari. `cancel` is
    /// what closing the sheet means.
    public func signIn(through session: PostioSession?) async -> Bool {
        guard let session else { return false }
        signingIn = true
        problem = nil
        let address = self.address
        let clientId = self.clientId
        let secret = clientSecret.isEmpty ? nil : clientSecret

        // Poll the boundary for the port while the flow runs: a person sent to Safari is asked to
        // trust that what waits is Postio, and the port is the only evidence a mail client can
        // offer.
        let watching = Task { @MainActor in
            while signingIn {
                signInMessage = session.signInProgress.message
                try? await Task.sleep(for: .milliseconds(250))
            }
        }

        let complaint = await Task.detached { [session] in
            session.signInWithBrowser(
                address: address, clientId: clientId, clientSecret: secret)
        }.value

        signingIn = false
        watching.cancel()
        signInMessage = ""
        problem = complaint
        return complaint == nil
    }

    public func cancelSignIn(through session: PostioSession?) {
        session?.cancelSignIn()
        signingIn = false
        signInMessage = ""
    }

    private func refreshHint() {
        hint = address.contains("@") ? providerHint(address: address) : nil
        if let hint { route = hint.route }
    }

    /// Empty when nothing is known: a guessed host dials somebody else's server.
    private func prefill() {
        guard let hint else { return }
        if imapHost.isEmpty { imapHost = hint.imapHost }
        if smtpHost.isEmpty { smtpHost = hint.smtpHost }
        imapPort = Int(hint.imapPort)
        smtpPort = Int(hint.smtpPort)
    }
}
