import AppKit
import Testing

@testable import PostioKit

/// The menu bar on screen is the one Postio built (#1262).
///
/// `MenuPlanTests` assert the plan; nothing asserted that the plan ever
/// reached `NSApp.mainMenu`, and for months it did not — SwiftUI rebuilds the
/// main menu from its own scenes, so the application ran with AppKit's stock
/// Edit menu, no Go, no Message, and every accelerator and context validation
/// that had been built and tested was invisible.
///
/// This is the third "built, tested, never mounted" bug in this port, after
/// the reader (#70) and the window that never appeared (#1146), so the
/// assertion is about mounting rather than about building.
@MainActor
// Serialized: the menu bar is application-global state and these two tests
// both install one. In parallel, one test's install looks to the other like
// the repair it was waiting for.
@Suite(.serialized) struct MenuBarMountedTests {
    /// Whether this process can reach the window server. A menu bar is
    /// application-global state, so this needs a real `NSApplication`.
    private var hasWindowServer: Bool { NSScreen.main != nil }

    private func install() {
        // The menu bar is `NSApplication`'s, and `NSApp` is nil in a process
        // that has never asked for one — which a test bundle has not.
        _ = NSApplication.shared
        MenuBar.install(
            bindings: { command in command == "reply" ? ["e", "cmd+r"] : [] },
            available: { _ in true },
            run: { _ in }
        )
    }

    @Test func theBarThatIsInstalledIsTheOneOnScreen() throws {
        try #require(
            windowServerVerdict(isCI: isCI, hasWindowServer: hasWindowServer) != .fail,
            "CI must have a window server; a skip here is indistinguishable from a pass"
        )
        try #require(hasWindowServer, "no window server: skipping, and saying so")

        install()

        #expect(MenuBar.isMounted)
        let titles = NSApp.mainMenu?.items.compactMap { $0.submenu?.title } ?? []
        #expect(titles.contains("Message"), "the registry's menus reach the bar: \(titles)")
    }

    @Test func aBarSwiftUiEditedInPlaceComesBack() async throws {
        try #require(
            windowServerVerdict(isCI: isCI, hasWindowServer: hasWindowServer) != .fail,
            "CI must have a window server; a skip here is indistinguishable from a pass"
        )
        try #require(hasWindowServer, "no window server: skipping, and saying so")

        install()
        let before = MenuBar.remounts

        // What SwiftUI actually does, measured on the running application: it
        // does not replace `mainMenu`, it edits the menu that is there —
        // dropping the submenus it does not know about. Identity was true
        // throughout, which is why the first two fixes for #1262 did nothing.
        let bar = try #require(NSApplication.shared.mainMenu)
        let message = try #require(bar.items.firstIndex { $0.submenu?.title == "Message" })
        bar.removeItem(at: message)

        #expect(NSApplication.shared.mainMenu === MenuBar.installed, "the object is still ours")
        #expect(!MenuBar.isMounted, "and the bar on screen is not: the bug, reproduced")

        // The rebuild is scheduled rather than immediate, so SwiftUI finishes
        // whatever edit it is part way through first.
        try await Task.sleep(for: .milliseconds(100))

        #expect(MenuBar.isMounted, "the bar has to come back")
        #expect(MenuBar.remounts > before, "and it came back by being rebuilt")
        let titles = NSApplication.shared.mainMenu?.items.compactMap { $0.submenu?.title } ?? []
        #expect(titles.contains("Message"), "with the menu that was taken out: \(titles)")
    }

    private var isCI: Bool {
        ProcessInfo.processInfo.environment["CI"] != nil
    }
}
