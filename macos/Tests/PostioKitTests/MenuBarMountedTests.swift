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

    @Test func theMountedEditMenuCanActuallyPaste() throws {
        // **The assertion that was missing.** `appendStandardEditing` can be
        // perfect and still never be called; what a user needs is a ⌘V on the
        // bar that is really on screen. Before the fix this menu existed,
        // held Postio's own commands, and had no Paste in it — so ⌘V did
        // nothing in every text field in the application, and every test
        // passed.
        try #require(
            windowServerVerdict(isCI: isCI, hasWindowServer: hasWindowServer) != .fail,
            "CI must have a window server; a skip here is indistinguishable from a pass"
        )
        try #require(hasWindowServer, "no window server: skipping, and saying so")

        install()

        let edit = NSApp.mainMenu?.items.compactMap(\.submenu).first { $0.title == "Edit" }
        let paste = try #require(
            edit?.items.first { $0.title == "Paste" },
            "the Edit menu on screen has no Paste, so ⌘V reaches no text field"
        )
        #expect(paste.keyEquivalent == "v")
        #expect(paste.keyEquivalentModifierMask == .command)
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

/// The editing items that make ⌘V work at all.
///
/// Replacing SwiftUI's menu bar (#1262, #1298) took its Edit menu with it, and
/// it every standard editing key equivalent — ⌘V, ⌘C, ⌘X, ⌘A, ⌘Z — across the
/// whole application. It is invisible from the code: a text field looks
/// perfectly focused and simply never receives the paste, because on this
/// platform ⌘V reaches a responder *through a menu item* and nowhere else.
///
/// Found by a password that could not be pasted into the add-account sheet,
/// after the key monitor had been cleared of it by a live probe: the monitor
/// passed ⌘V through correctly and there was nothing underneath to catch it.
@MainActor
@Suite struct StandardEditingItemsTests {
    private func editMenu() -> NSMenu {
        let menu = NSMenu()
        MenuBar.appendStandardEditing(to: menu)
        return menu
    }

    @Test func pasteCarriesCommandV() {
        // The one this exists for.
        let paste = editMenu().items.first { $0.title == "Paste" }

        #expect(paste?.keyEquivalent == "v")
        #expect(paste?.keyEquivalentModifierMask == .command)
        #expect(paste?.action == #selector(NSText.paste(_:)))
    }

    @Test func everyStandardEditingItemIsThereWithItsUsualKey() {
        let items = editMenu().items.filter { !$0.isSeparatorItem }
        let byTitle = Dictionary(uniqueKeysWithValues: items.map { ($0.title, $0) })

        #expect(byTitle["Cut"]?.keyEquivalent == "x")
        #expect(byTitle["Copy"]?.keyEquivalent == "c")
        #expect(byTitle["Select All"]?.keyEquivalent == "a")
        #expect(byTitle["Undo"]?.keyEquivalent == "z")
        #expect(byTitle["Redo"]?.keyEquivalent == "z")
        #expect(
            byTitle["Redo"]?.keyEquivalentModifierMask == [.command, .shift],
            "Redo is shift-command-Z; plain command-Z is Undo"
        )
    }

    @Test func theSelectorsGoDownTheResponderChain() {
        // `target == nil` is what makes an item enable itself only when
        // something focused can perform it. An item wired to a fixed target
        // would be live with no text field in front and paste into nothing.
        for item in editMenu().items where !item.isSeparatorItem {
            #expect(item.target == nil, "\(item.title) is aimed at a fixed target")
        }
    }

    @Test func noneOfThemCollidesWithAPostioBinding() {
        // Postio's modified defaults are `ctrl+…`, which is ⌃ on this platform
        // and not ⌘, so these five chords are unclaimed. If a future binding
        // takes one, the monitor still wins — it runs before menu key
        // equivalents — but the menu would then draw a key that never fires,
        // which is the lie this catches.
        let taken = Set(
            PostioRegistry.commands.flatMap { [$0.defaultBinding] + $0.alternateBindings }
        )
        for item in editMenu().items where !item.isSeparatorItem {
            let chord = "cmd+\(item.keyEquivalent)"
            #expect(!taken.contains(chord), "\(item.title) collides with \(chord)")
        }
    }
}

/// ⌘W closes a window, which needs a menu item like everything else.
///
/// The same shape as the missing Paste (#1298): on this platform a window
/// closes on that chord *through a menu item* and nowhere else, so replacing
/// SwiftUI's bar took Close away with Edit, and the Settings window could not
/// be closed from the keyboard at all. Reported from real use, and visible in
/// this session's own transcript — a ⌘W sent to the Settings window did
/// nothing and it was not noticed at the time.
@MainActor
@Suite(.serialized) struct CloseWindowTests {
    private var hasWindowServer: Bool { NSScreen.main != nil }
    private var isCI: Bool { ProcessInfo.processInfo.environment["CI"] != nil }

    @Test func theMountedBarCanCloseAWindow() throws {
        try #require(
            windowServerVerdict(isCI: isCI, hasWindowServer: hasWindowServer) != .fail,
            "CI must have a window server; a skip here is indistinguishable from a pass"
        )
        try #require(hasWindowServer, "no window server: skipping, and saying so")

        _ = NSApplication.shared
        MenuBar.install(bindings: { _ in [] }, available: { _ in true }, run: { _ in })

        let items = NSApp.mainMenu?.items.compactMap(\.submenu).flatMap(\.items) ?? []
        let close = try #require(
            items.first { $0.title == "Close" },
            "no Close item anywhere on the bar, so ⌘W closes nothing"
        )
        #expect(close.keyEquivalent == "w")
        #expect(close.keyEquivalentModifierMask == .command)
        #expect(close.action == #selector(NSWindow.performClose(_:)))
        #expect(close.target == nil, "Close goes down the responder chain to the key window")
    }
}
