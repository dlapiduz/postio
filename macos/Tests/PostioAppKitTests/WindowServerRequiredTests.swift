@testable import PostioAppKit
import AppKit
import Testing

enum WindowServerVerdict: Equatable {
    case fine
    case skipAndSay
    case fail
}

func windowServerVerdict(isCI: Bool, hasWindowServer: Bool) -> WindowServerVerdict {
    switch (isCI, hasWindowServer) {
    case (_, true): return .fine
    case (false, false): return .skipAndSay
    case (true, false): return .fail
    }
}

@Suite struct WindowServerRequiredTests {
    private var hasWindowServer: Bool {
        NSScreen.main != nil
    }

    @Test func theVerdictFailsOnlyWhereASkipWouldLie() {
        #expect(windowServerVerdict(isCI: true, hasWindowServer: true) == .fine)
        #expect(windowServerVerdict(isCI: false, hasWindowServer: true) == .fine)
        #expect(windowServerVerdict(isCI: false, hasWindowServer: false) == .skipAndSay)
        #expect(
            windowServerVerdict(isCI: true, hasWindowServer: false) == .fail,
            "a skip in CI is indistinguishable from a pass, which is the point"
        )
    }

    @Test func ciHasAWindowServerToRunTheSwiftSuitesOn() throws {
        let verdict = windowServerVerdict(
            isCI: ProcessInfo.processInfo.environment["CI"] != nil,
            hasWindowServer: hasWindowServer
        )
        if verdict == .skipAndSay {
            print(
                "no window server: any Swift test that needed one would be "
                    + "skipping here, which is correct locally"
            )
        }
        #expect(
            verdict != .fail,
            """
            CI has no window server, so any Swift test that needs one has \
            been skipping and reporting success. This is not a failure of the \
            code under test; it is the session. A macOS runner has an Aqua \
            session by default — an ssh step, a `launchctl` context without \
            one, or a job running as a different user does not. Check that \
            scripts/macos-test.sh is being run by the runner's own user in \
            its own session, the way `runs-on: macos-latest` gives you.
            """
        )
    }
}
