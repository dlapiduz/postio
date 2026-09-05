import AppKit
import Testing

/// A skip that cannot pass for a test.
///
/// `postio-gtk`'s `gtk_display_required.rs` exists because around 117 test
/// files opened with "no display? skip and return", which is right on a
/// contributor's headless shell and wrong in CI: with no display every one of
/// them returned early and reported success, including the accessibility audit
/// `docs/PRODUCT.md` §20 depends on (#114). *"A skip that is right locally and
/// wrong in CI has to know which one it is in. A skip nobody can distinguish
/// from a pass is not a test."*
///
/// The Swift suites deliberately need no window server today — every decision
/// they assert is a pure function, which is why `MenuPlan`, `Announcements`
/// and `PaletteRow` exist as separate types at all. But the moment one does,
/// the same trap is open, and on this platform it is easier to fall into: an
/// ssh session has no `Aqua` session, `NSApplication` cannot reach the window
/// server, and an `XCTSkip` in that situation looks exactly like a pass.
///
/// So this is the guard, written before it is needed, for the same reason the
/// GTK one is a single test rather than 117 edits.
enum WindowServerVerdict: Equatable {
    /// There is a window server; anything that needs one can run.
    case fine
    /// There is none, and that is expected here. Say so, do not fail.
    case skipAndSay
    /// There is none and there should be. A skip here is indistinguishable
    /// from a pass, which is the whole failure this guards.
    case fail
}

/// What to do about a missing window server, given where we are.
///
/// A free function over two booleans, because the situation that has to fail
/// is the one this machine cannot be put into: a Mac with a window server
/// cannot demonstrate the CI-without-one case, and a guard whose failing
/// branch has never been seen fail is exactly the untested skip it exists to
/// prevent. So the *decision* is asserted for all four combinations and the
/// live test below is one line that reads the environment.
func windowServerVerdict(isCI: Bool, hasWindowServer: Bool) -> WindowServerVerdict {
    switch (isCI, hasWindowServer) {
    case (_, true): return .fine
    // A contributor's shell. Skipping is correct — but say so, so a local run
    // that is headless is visible in the output rather than silent.
    case (false, false): return .skipAndSay
    case (true, false): return .fail
    }
}

@Suite struct WindowServerRequiredTests {
    /// Whether this process can reach the window server.
    ///
    /// `NSApp` is not the question — an app object exists in any process. The
    /// question is whether there is a session to draw into, which is what
    /// `NSScreen.main` answers and what an ssh session lacks.
    private var hasWindowServer: Bool {
        NSScreen.main != nil
    }

    @Test func theVerdictFailsOnlyWhereASkipWouldLie() {
        // The four cases, including the one this machine cannot be put into.
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
