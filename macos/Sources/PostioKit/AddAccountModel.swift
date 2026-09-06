import Foundation
import PostioFFI

/// The three steps of adding an account (#1279, canvas screen 27).
///
/// The flow is a model rather than a pile of view state because the parts
/// worth being sure about are decisions: which route an address suggests,
/// when `Continue` may be pressed, and what the step counter says. None of
/// them needs a window to be asserted, and a sheet checked only by clicking
/// through it is a sheet nobody checks.
@MainActor
@Observable
public final class AddAccountModel: Identifiable {
    /// So a `sheet(item:)` can present it.
    public let id = UUID()

    /// Where the flow is. Three steps, counted from one in the corner.
    public enum Step: Int, CaseIterable {
        case address = 1
        case credentials = 2
        case store = 3
    }

    /// How a store is laid out on disk.
    public enum Format: String, CaseIterable, Identifiable {
        case maildir, mbox, notmuch
        public var id: String { rawValue }
    }

    public private(set) var step: Step = .address

    /// Step 1.
    public var address = "" {
        didSet { refreshHint() }
    }
    public private(set) var hint: ProviderHintFfi?
    public var route: RouteFfi = .imap

    /// Step 2.
    public var password = ""
    public var imapHost = ""
    public var imapPort = 993
    public var smtpHost = ""
    public var smtpPort = 465

    /// Step 3.
    public var storePath: String
    public var format: Format = .maildir
    /// How far back to sync, in days. `0` means everything.
    public var syncDays = 90

    /// Anything that went wrong, in words for the person reading them.
    public private(set) var problem: String?

    public init(storePath: String = "~/mail") {
        self.storePath = storePath
    }

    /// The counter in the corner: `1 of 3`.
    public var counter: String { "\(step.rawValue) of \(Step.allCases.count)" }

    /// What the verdict strip says, before an address has been typed and
    /// after.
    public var verdict: String {
        hint?.verdict ?? "Type an email address to begin."
    }

    /// Whether `Continue` does anything yet.
    ///
    /// Step 1 needs something that looks like an address — the check is
    /// deliberately shallow, because the authority on whether an address
    /// exists is the server, and a client that refuses valid addresses out of
    /// strictness is worse than one that tries and reports.
    public var canContinue: Bool {
        switch step {
        case .address: address.contains("@") && !address.hasSuffix("@")
        case .credentials:
            switch route {
            case .imap: !password.isEmpty && !imapHost.isEmpty && !smtpHost.isEmpty
            // The sign-in happens in the browser, so there is nothing to fill
            // in here and nothing to check.
            case .outlook, .gmail, .localStore: true
            }
        case .store: !storePath.isEmpty
        }
    }

    /// The estimate under the sync window: `about 4,000 messages · 1.2 GB`.
    ///
    /// Deliberately vague and deliberately present. Nobody can know before
    /// syncing, and a person choosing "everything" for a fifteen-year mailbox
    /// should be told what they are asking for in the units they will feel it
    /// in — disk, and time.
    public var estimate: String {
        switch syncDays {
        case 0: "everything on the server · however long that takes"
        case ...30: "about a month · minutes"
        case ...90: "about three months · minutes"
        case ...365: "about a year · tens of minutes"
        default: "several years · possibly hours"
        }
    }

    /// Move on, if there is anywhere to move to.
    public func next() {
        guard canContinue else { return }
        problem = nil
        switch step {
        case .address:
            route = hint?.route ?? .imap
            prefill()
            step = .credentials
        case .credentials: step = .store
        case .store: break
        }
    }

    /// Back one step. The first step's back is Cancel, which the sheet owns.
    public func back() {
        problem = nil
        step = Step(rawValue: step.rawValue - 1) ?? .address
    }

    /// Finish: write the account, and answer whether the sheet may close.
    public func finish(through session: PostioSession?) -> Bool {
        switch route {
        case .imap:
            guard let session else { return false }
            if let complaint = session.addImapAccount(
                address: address,
                password: password,
                imapHost: imapHost,
                imapPort: UInt16(imapPort),
                smtpHost: smtpHost,
                smtpPort: UInt16(smtpPort)
            ) {
                problem = complaint
                return false
            }
            return true
        case .outlook, .gmail:
            // Said rather than faked. Consent has to happen in the system
            // browser and the token has to reach the Keychain (ADR 0006 Q3),
            // and neither is wired here yet — a sheet that closed as though
            // an account had been added would leave somebody waiting for mail
            // that is never coming (#1276).
            problem =
                "Signing in through the browser is not built here yet. "
                + "Choose IMAP / SMTP to add this account with a password."
            return false
        case .localStore:
            problem =
                "Opening a store that already exists is not built here yet (#1278)."
            return false
        }
    }

    /// What the preset table says about the address typed so far.
    private func refreshHint() {
        hint = address.contains("@") ? providerHint(address: address) : nil
        if let hint { route = hint.route }
    }

    /// Fill the server fields from the preset, leaving them empty when
    /// nothing is known — a guessed host dials somebody else's server.
    private func prefill() {
        guard let hint else { return }
        if imapHost.isEmpty { imapHost = hint.imapHost }
        if smtpHost.isEmpty { smtpHost = hint.smtpHost }
        imapPort = Int(hint.imapPort)
        smtpPort = Int(hint.smtpPort)
    }
}
