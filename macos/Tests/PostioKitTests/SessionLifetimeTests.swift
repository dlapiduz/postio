import Testing

@testable import PostioKit

/// When Postio is allowed to stop collecting mail.
///
/// The session was ended on `ScenePhase.background`, which on this platform
/// is what `⌘W`, `⌘H` and minimising all produce. So the gesture a Mac user
/// makes to *leave a mail client running* — put the window away and let it
/// collect — was the gesture that stopped every sync, every IDLE connection
/// and every new-mail notification. Reopening a window found the engine
/// `.open` with no session behind it: a full mailbox drawing "No messages",
/// no keyboard, and no way back short of quitting.
///
/// The reason recorded for doing it was that the store was SQLCipher and
/// dropping an engine at process exit was "exactly when libcrypto goes away
/// underneath a thread still encrypting a page". That store is Turso now.
/// The reason went with it.
@Suite struct SessionLifetimeTests {
    @Test func puttingTheWindowAwayDoesNotStopTheMail() {
        // The whole point of a mail client in the Dock.
        #expect(!SessionLifetime.shouldEnd(on: .background))
        #expect(!SessionLifetime.shouldEnd(on: .inactive))
        #expect(!SessionLifetime.shouldEnd(on: .active))
    }

    @Test func quittingEndsIt() {
        // The one moment it is right: there is nothing left to collect mail
        // for, and the store would rather be closed than dropped.
        #expect(SessionLifetime.shouldEnd(on: .terminating))
    }

    @Test func everyPhaseHasAnAnswer() {
        // An unhandled phase that defaulted to "stop" would be this bug
        // again, one macOS release later.
        for phase in SessionPhase.allCases {
            _ = SessionLifetime.shouldEnd(on: phase)
        }
    }
}
