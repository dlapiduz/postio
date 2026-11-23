import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

@MainActor
// Serialized: the menu bar is application-global state, and in parallel one test's install looks
// like the repair the other waits for.
@Suite(.serialized) struct MenuBarMountedTests {
    private var hasWindowServer: Bool { NSScreen.main != nil }

    private func install() {
        // `NSApp` is nil in a process that never asked for one, as a test bundle has not.
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
        // Asserts on the mounted bar: `appendStandardEditing` can be right and never called, which
        // left ⌘V dead in every text field while tests passed.
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

        // SwiftUI edits the existing `mainMenu`, dropping submenus it does not know, rather than
        // replacing it; identity held throughout (#1262).
        let bar = try #require(NSApplication.shared.mainMenu)
        let message = try #require(bar.items.firstIndex { $0.submenu?.title == "Message" })
        bar.removeItem(at: message)

        #expect(NSApplication.shared.mainMenu === MenuBar.installed, "the object is still ours")
        #expect(!MenuBar.isMounted, "and the bar on screen is not: the bug, reproduced")

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

@MainActor
@Suite struct StandardEditingItemsTests {
    private func editMenu() -> NSMenu {
        let plan = MenuPlan.bar(bindings: { _ in [] })
        let edit = plan.first { $0.section == .edit }!
        return MenuBar.menu(for: edit, target: nil, undo: nil)
    }

    private func standardItems() -> [NSMenuItem] {
        editMenu().items.filter { !$0.isSeparatorItem && $0.representedObject == nil }
    }

    @Test func pasteCarriesCommandV() {
        let paste = editMenu().items.first { $0.title == "Paste" }

        #expect(paste?.keyEquivalent == "v")
        #expect(paste?.keyEquivalentModifierMask == .command)
        #expect(paste?.action == #selector(NSText.paste(_:)))
    }

    @Test func everyStandardEditingItemIsThereWithItsUsualKey() {
        let items = standardItems()
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
        for item in standardItems() {
            #expect(item.target == nil, "\(item.title) is aimed at a fixed target")
        }
    }

    @Test func aChordPostioAlsoBindsIsHandedToAField() {
        // The key monitor runs before menu key equivalents, so a chord like ⌘A reaches a text field
        // only if `KeyDisposition.belongsToText` lets it through; otherwise Select All in search
        // would select list rows.
        let taken = Set(
            PostioRegistry.commands.flatMap { [$0.defaultBinding] + $0.alternateBindings }
        )
        for item in standardItems() where item.keyEquivalentModifierMask == .command {
            guard taken.contains("mod+\(item.keyEquivalent)") else { continue }
            let key = KeyEvent.Reduced(
                character: item.keyEquivalent, name: nil,
                modifiers: ModifiersFfi(control: false, option: false, shift: false, command: true))
            #expect(
                KeyDisposition.belongsToText(key, typing: true),
                "\(item.title) is Postio's too, and a field never gets it")
        }
    }
}

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
