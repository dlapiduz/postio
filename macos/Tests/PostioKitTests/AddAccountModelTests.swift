import PostioFFI
import Testing

@testable import PostioKit

@MainActor
@Suite struct AddAccountModelTests {
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
        let model = AddAccountModel()
        #expect(model.verdict.contains("Type an email address"))

        model.address = unknown
        let hint = providerHint(address: unknown)

        #expect(model.verdict == hint.verdict)
        #expect(model.route == hint.route)
    }

    @Test func anUnknownDomainOffersImapAndFillsInNoServers() {
        // A guess that dials someone else's server is what this must not do.
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

    @Test func aRouteSomebodyPickedIsTheRouteTheyGet() {
        // The address suggests a route; a click on another is the person deciding, so `next` must
        // not put the suggestion back.
        let model = AddAccountModel()
        model.address = unknown
        model.route = .localStore
        model.next()

        #expect(model.route == .localStore)
        #expect(model.canContinue, "a local store has no credential to wait for")
    }

    @Test func aLocalStoreHasNoCredentialStepToShow() {
        let model = AddAccountModel()
        model.address = unknown
        model.route = .localStore
        #expect(model.counter == "1 of 2")
        model.next()
        #expect(model.step == .store)
        #expect(model.counter == "2 of 2")
        model.back()
        #expect(model.step == .address, "and Back returns to the address, not to an empty page")
    }

    @Test func typingAnotherAddressSuggestsAgain() {
        let model = AddAccountModel()
        model.route = .localStore
        model.address = unknown
        #expect(model.route == providerHint(address: unknown).route)
    }

    @Test func goingBackKeepsWhatWasTyped() {
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
        let model = AddAccountModel()
        model.syncDays = 0
        #expect(model.estimate.contains("everything"))

        model.syncDays = 30
        #expect(model.estimate.contains("month"))
    }

    @Test func anOauthRouteIsFinishedBySigningIn_notByStartSync() async {
        // An OAuth route waits on someone in another application, so pressing on without signing in
        // says where the sign-in is rather than closing over an account never added.
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.route = .gmail
        model.clientId = "the-users-own-client"
        model.next()

        let finished = await model.finish(through: nil)
        #expect(finished == false)
        #expect(model.problem?.contains("browser") == true)
    }

    @Test func theOnlyFormatOfferedIsTheOnePostioOpens() {
        // A picker that offers what the app cannot do reads as a choice until made (#1296).
        #expect(AddAccountModel.Format.allCases == [.maildir])
    }

    @Test func nothingIsSaidAboutAStorePathOnARouteThatHasNoStore() {
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.route = .imap

        #expect(model.storeProblem(through: nil) == nil)
    }

    @Test func anOauthRouteWaitsForTheClientIdThatPostioDoesNotShip() {
        // A client id inside an open-source app is shared by every user, and a provider that
        // notices revokes it for all (ADR 0006 Q1).
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.route = .gmail

        #expect(!model.canContinue, "there is nothing to sign in with yet")

        model.clientId = "the-users-own-client"
        #expect(model.canContinue)
    }

    @Test func aClientIdOfNothingButSpacesIsNotAClientId() {
        let model = AddAccountModel()
        model.address = unknown
        model.next()
        model.route = .gmail
        model.clientId = "   "

        #expect(!model.canContinue)
    }

    @Test func nothingIsSigningInBeforeAnybodyPressesSignIn() {
        let model = AddAccountModel()
        #expect(!model.signingIn)
        #expect(model.signInMessage.isEmpty)
    }

    @Test func cancellingASignInStopsWaitingEvenWithNoSession() {
        let model = AddAccountModel()
        model.cancelSignIn(through: nil)

        #expect(!model.signingIn)
        #expect(model.signInMessage.isEmpty)
    }
}
