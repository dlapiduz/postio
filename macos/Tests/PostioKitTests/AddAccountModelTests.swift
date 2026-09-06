import PostioFFI
import Testing

@testable import PostioKit

/// The three steps of adding an account (#1279).
@MainActor
@Suite struct AddAccountModelTests {
    @Test func theCounterSaysWhereYouAre() {
        let model = AddAccountModel()
        #expect(model.counter == "1 of 3")

        model.address = "mara@gmail.com"
        model.next()
        #expect(model.counter == "2 of 3")
    }

    @Test func continueDoesNothingUntilThereIsAnAddress() {
        let model = AddAccountModel()
        #expect(!model.canContinue)

        model.address = "mara@"
        #expect(!model.canContinue, "an address that stops at the @ is not one")

        model.address = "mara@gmail.com"
        #expect(model.canContinue)
    }

    @Test func theVerdictNamesWhatTheAddressWasRecognisedAs() {
        let model = AddAccountModel()
        #expect(model.verdict.contains("Type an email address"))

        model.address = "mara@gmail.com"
        #expect(model.verdict.lowercased().contains("gmail")
            || model.verdict.lowercased().contains("google"))
        #expect(model.route == .gmail, "and the route it recognised is pre-selected")
    }

    @Test func anUnknownDomainOffersImapAndFillsInNoServers() {
        // The guess that dials somebody else's server is the one thing this
        // must not do.
        let model = AddAccountModel()
        model.address = "ada@ostwald.invalid"
        model.next()

        #expect(model.route == .imap)
        #expect(model.imapHost.isEmpty)
        #expect(model.smtpHost.isEmpty)
        #expect(!model.canContinue, "and Continue waits for the servers")
    }

    @Test func aKnownProviderFillsInItsOwnServers() {
        let model = AddAccountModel()
        model.address = "mara@gmail.com"
        model.next()

        #expect(!model.imapHost.isEmpty)
        #expect(model.imapPort > 0)
    }

    @Test func goingBackKeepsWhatWasTyped() {
        // Losing a typed address on Back is the kind of small cruelty that
        // makes people abandon a setup flow.
        let model = AddAccountModel()
        model.address = "ada@ostwald.invalid"
        model.next()
        model.back()

        #expect(model.step == .address)
        #expect(model.address == "ada@ostwald.invalid")
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
        model.address = "mara@gmail.com"
        model.next()
        model.next()

        #expect(model.finish(through: nil) == false)
        #expect(model.problem?.contains("not built here yet") == true)
    }
}
