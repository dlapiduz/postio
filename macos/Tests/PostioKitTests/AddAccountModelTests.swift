import PostioFFI
import Testing

@testable import PostioKit

/// The three steps of adding an account (#1279).
///
/// Every case here uses a reserved domain, and none names a provider. That
/// is not only the fixture rule: what this suite proves is that the model
/// mirrors whatever the **preset table** answered, and a test written around
/// one provider would prove that one row exists instead. Which providers are
/// recognised is the table's own business, asserted in `postio-ffi`.
@MainActor
@Suite struct AddAccountModelTests {
    /// A domain no preset claims — the ordinary case for a self-hosted
    /// address, and the one where guessing a server would be a real fault.
    private let unknown = "ada@ostwald.invalid"

    @Test func theCounterSaysWhereYouAre() {
        let model = AddAccountModel()
        #expect(model.counter == "1 of 3")

        model.address = unknown
        model.next()
        #expect(model.counter == "2 of 3")
    }

    @Test func continueDoesNothingUntilThereIsAnAddress() {
        let model = AddAccountModel()
        #expect(!model.canContinue)

        model.address = "ada@"
        #expect(!model.canContinue, "an address that stops at the @ is not one")

        model.address = unknown
        #expect(model.canContinue)
    }

    @Test func theVerdictAndTheRouteAreTheBoundarysAnswer() {
        // Not this frontend's: the sheet draws what the preset table said,
        // and a Swift-side opinion about which provider an address belongs to
        // would be the branch-per-provider the product forbids.
        let model = AddAccountModel()
        #expect(model.verdict.contains("Type an email address"))

        model.address = unknown
        let hint = providerHint(address: unknown)

        #expect(model.verdict == hint.verdict)
        #expect(model.route == hint.route)
    }

    @Test func anUnknownDomainOffersImapAndFillsInNoServers() {
        // The guess that dials somebody else's server is the one thing this
        // must not do.
        let model = AddAccountModel()
        model.address = unknown
        model.next()

        #expect(model.route == .imap)
        #expect(model.imapHost.isEmpty)
        #expect(model.smtpHost.isEmpty)
        #expect(!model.canContinue, "and Continue waits for the servers")
    }

    @Test func theServerFieldsAreFilledFromWhateverTheTableAnswered() {
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        let hint = providerHint(address: unknown)

        #expect(model.imapHost == hint.imapHost)
        #expect(model.imapPort == Int(hint.imapPort))
        #expect(model.smtpPort == Int(hint.smtpPort))
        #expect(model.imapPort > 0, "a port field showing 0 asks an unanswerable question")
    }

    @Test func goingBackKeepsWhatWasTyped() {
        // Losing a typed address on Back is the kind of small cruelty that
        // makes people abandon a setup flow.
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.back()

        #expect(model.step == .address)
        #expect(model.address == unknown)
    }

    @Test func backFromTheFirstStepGoesNowhere() {
        let model = AddAccountModel()
        model.back()
        #expect(model.step == .address)
    }

    @Test func theSyncWindowSaysWhatItWillCost() {
        // A person choosing "everything" for a fifteen-year mailbox should be
        // told what they are asking for, in the units they will feel it in.
        let model = AddAccountModel()
        model.syncDays = 0
        #expect(model.estimate.contains("everything"))

        model.syncDays = 30
        #expect(model.estimate.contains("month"))
    }

    @Test func aRouteThatIsNotBuiltSaysSoRatherThanClosing() {
        // A sheet that closed as though an account had been added would leave
        // somebody waiting for mail that is never coming.
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.route = .gmail
        model.next()

        #expect(model.finish(through: nil) == false)
        #expect(model.problem?.contains("not built here yet") == true)
    }

    @Test func aLocalStoreSaysTheSameThing() {
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.route = .localStore
        model.next()

        #expect(model.finish(through: nil) == false)
        #expect(model.problem?.contains("not built here yet") == true)
    }
}
