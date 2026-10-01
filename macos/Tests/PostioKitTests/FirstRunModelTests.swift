import PostioFFI
import Testing

@testable import PostioKit

/// The first-run wizard (canvas 09): an address, a password, what was found,
/// and `Connect`, which signs in before anything is written.
///
/// The boundary is stood in for by three closures, so what is asserted is
/// the model's decisions -- when to look an address up, when `Connect` may
/// be pressed, what a refusal does to the screen -- and not the network.
/// That the boundary's answers are the desktop's is `postio-ffi`'s
/// `first_run` suite.
@MainActor
@Suite struct FirstRunModelTests {
    private let address = "ada@ostwald.invalid"

    /// A boundary that answers `card` to every lookup and `complaint` to
    /// every connect, and remembers what it was asked.
    @MainActor
    final class Boundary {
        var card: DiscoveredFfi
        var complaint: String?
        var lookups: [String] = []
        var connected: [NewAccountFfi] = []
        var windows: [SyncWindowFfi] = []

        init(card: DiscoveredFfi, complaint: String? = nil) {
            self.card = card
            self.complaint = complaint
        }

        func model() -> FirstRunModel {
            FirstRunModel(
                choices: syncWindowChoices(),
                lookUp: { address in
                    self.lookups.append(address)
                    return self.card
                },
                connect: { account in
                    self.connected.append(account)
                    return self.complaint
                },
                writeWindow: { window in
                    self.windows.append(window)
                    return nil
                }
            )
        }
    }

    private static func server(_ host: String, _ port: UInt16) -> ServerFfi {
        ServerFfi(host: host, port: port, security: .tls)
    }

    private static func found(_ domain: String = "ostwald.invalid") -> DiscoveredFfi {
        DiscoveredFfi(
            found: true,
            heading: "Found settings for \(domain)",
            imap: server("imap.\(domain)", 993),
            smtp: server("smtp.\(domain)", 465),
            imapLine: "imap.\(domain):993 · TLS",
            smtpLine: "smtp.\(domain):465 · TLS",
            login: "ada@\(domain)",
            requiresAppPassword: false,
            note: nil,
            helpUrl: nil,
            source: "autoconfig",
            browserSignIn: false
        )
    }

    private static func guessed() -> DiscoveredFfi {
        var card = found()
        card.found = false
        card.heading = "Postio guessed the settings for ostwald.invalid. Check them before connecting."
        return card
    }

    private static func nothing() -> DiscoveredFfi {
        var card = guessed()
        card.imap = server("", 993)
        card.smtp = server("", 465)
        card.imapLine = ""
        card.smtpLine = ""
        return card
    }

    @Test func typingAnAddressLooksNothingUp() {
        // Each lookup is DNS and HTTPS to the address's domain. Per keystroke
        // that would announce every prefix of somebody's address to whoever
        // answers for `a`, `ad`, `ada@o`...
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        for prefix in ["a", "ad", "ada@", "ada@ostwald", address] {
            model.address = prefix
        }
        #expect(boundary.lookups.isEmpty)
    }

    @Test func leavingTheFieldLooksItUpOnceAndFillsTheCard() async {
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        model.address = address

        await model.lookUp()
        await model.lookUp()

        #expect(boundary.lookups == [address], "the same address is not asked about twice")
        #expect(model.discovered?.found == true)
        #expect(model.imapHost == "imap.ostwald.invalid")
        #expect(model.smtpHost == "smtp.ostwald.invalid")
        #expect(!model.editing, "found settings are shown, not handed over as a form")
    }

    @Test func somethingThatIsNotAnAddressIsNotLookedUp() async {
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        model.address = "ada@ostwald"

        await model.lookUp()

        #expect(boundary.lookups.isEmpty)
        #expect(model.discovered == nil)
    }

    @Test func aGuessOpensTheServersForChecking() async {
        // postio-69: a custom domain gets the common names to start from, in
        // an open form, never the "Found settings" card.
        let boundary = Boundary(card: Self.guessed())
        let model = boundary.model()
        model.address = address

        await model.lookUp()

        #expect(model.editing)
        #expect(model.imapHost == "imap.ostwald.invalid", "prefilled, for the person to check")
    }

    @Test func changingTheAddressTakesTheCardAway() async {
        // A card for the old domain under a new address is a claim about
        // the wrong servers, and Connect would dial them.
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        model.address = address
        await model.lookUp()

        model.address = "ada@hollis.invalid"

        #expect(model.discovered == nil)
        #expect(model.imapHost.isEmpty)
        #expect(model.login == "ada@hollis.invalid", "the login follows the address it was")
    }

    @Test func connectNeedsAnAddressAPasswordAndServers() async {
        let boundary = Boundary(card: Self.nothing())
        let model = boundary.model()
        #expect(!model.canConnect)

        model.address = address
        model.password = "hunter2"
        #expect(model.canConnect, "not looked up yet: Connect looks it up first")

        await model.lookUp()
        #expect(!model.canConnect, "nothing found and nothing typed: no server to sign in to")

        model.imapHost = "imap.ostwald.invalid"
        model.smtpHost = "smtp.ostwald.invalid"
        #expect(model.canConnect)
    }

    @Test func connectLooksTheAddressUpWhenNobodyLeftTheField() async {
        // Return pressed straight from the password field: the lookup has
        // not run, and signing in to empty servers is not the answer.
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        model.address = address
        model.password = "hunter2"

        await model.connect()

        #expect(boundary.lookups == [address])
        #expect(boundary.connected.count == 1)
        #expect(boundary.connected.first?.imap.host == "imap.ostwald.invalid")
        #expect(boundary.connected.first?.password == "hunter2")
        #expect(model.phase == .syncWindow)
        #expect(model.counter == "3 of 3")
    }

    @Test func aRefusedLoginSaysWhyAndStaysOnTheForm() async {
        let boundary = Boundary(card: Self.found(), complaint: "The server refused that password.")
        let model = boundary.model()
        model.address = address
        model.password = "wrong"

        await model.connect()

        #expect(model.phase == .account)
        #expect(model.problem == "The server refused that password.")
        #expect(model.address == address, "nothing typed is thrown away")
    }

    @Test func typingAgainAfterARefusalClearsTheComplaint() async {
        let boundary = Boundary(card: Self.found(), complaint: "The server refused that password.")
        let model = boundary.model()
        model.address = address
        model.password = "wrong"
        await model.connect()

        model.password = "right"

        #expect(model.problem == nil)
    }

    @Test func aProviderWhoseDoorIsTheBrowserIsNotOfferedAPasswordConnect() async {
        var card = Self.found()
        card.browserSignIn = true
        let boundary = Boundary(card: card)
        let model = boundary.model()
        model.address = address
        model.password = "hunter2"
        await model.lookUp()

        #expect(model.signsInWithBrowser)
        #expect(!model.canConnect)
    }

    @Test func startingTheSyncWritesTheWindowChosenAndFinishes() async {
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        model.address = address
        model.password = "hunter2"
        await model.connect()
        #expect(model.window == .lastYear, "the field's own default is preselected")

        model.window = .lastMonth
        model.startSync()

        #expect(boundary.windows == [.lastMonth])
        #expect(model.phase == .finished)
    }

    @Test func theCounterFollowsTheSharedSteps() async {
        let boundary = Boundary(card: Self.found())
        let model = boundary.model()
        #expect(model.counter == "1 of 3")
    }
}
