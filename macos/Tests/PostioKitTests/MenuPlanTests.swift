import PostioFFI
import Testing

@testable import PostioKit

@Suite struct MenuPlanTests {
    private func defaults(_ command: String) -> [String] {
        guard let spec = PostioRegistry.commands.first(where: { $0.id == command }) else {
            return []
        }
        return ([spec.defaultBinding] + spec.alternateBindings)
            .map { $0.replacingOccurrences(of: "mod+", with: "cmd+") }
    }

    @Test func theMenuBarIsBuiltFromTheRegistry() {
        let bar = MenuPlan.build(bindings: defaults)
        #expect(!bar.isEmpty, "no menus at all")

        let known = Set(PostioRegistry.commands.map(\.id))
        for menu in bar {
            for item in menu.items {
                let command = item.command ?? "nothing"
                #expect(known.contains(command), "\(command) is not in the registry")
            }
        }
    }

    @Test func aCommandGoesUnderTheMenuTheCoreChose() {
        let bar = MenuPlan.build(bindings: defaults)
        let message = bar.first { $0.title == "Message" }
        #expect(message?.items.contains { $0.command == "archive" } == true)
        let file = bar.first { $0.title == "File" }
        #expect(file?.items.contains { $0.command == "archive" } == false)
    }

    @Test func noMenuIsDrawnEmpty() {
        for menu in MenuPlan.build(bindings: { _ in [] }) {
            #expect(!menu.items.isEmpty, "\(menu.title) is empty")
        }
    }

    @Test func theAcceleratorIsWhatIsBoundNotWhatTheRegistryDefaultsTo() {
        // A menu drawing the default for a rebound command is confidently wrong.
        let bar = MenuPlan.build(bindings: { $0 == "archive" ? ["ctrl+shift+e"] : [] })
        let archive = bar.flatMap(\.items).first { $0.command == "archive" }
        #expect(archive?.shortcut == "⌃⇧E")
    }

    @Test func modifiersAreDrawnInApplesOrderNotTheBindingsOrder() {
        #expect(MenuPlan.accelerator(from: "cmd+shift+alt+ctrl+k") == "⌃⌥⇧⌘K")
        #expect(MenuPlan.accelerator(from: "shift+cmd+a") == "⇧⌘A")
        #expect(MenuPlan.accelerator(from: "cmd+k") == "⌘K")
    }

    @Test func aNamedKeyIsDrawnAsItsGlyph() {
        #expect(MenuPlan.accelerator(from: "cmd+Return") == "⌘↩")
        #expect(MenuPlan.accelerator(from: "Escape") == "⎋")
        #expect(MenuPlan.accelerator(from: "shift+Tab") == "⇧⇥")
    }

    @Test func aShiftFoldedCapitalKeepsItsShift() {
        // The resolver folds Shift into the character (`shift+a` is `A`), so uppercasing for
        // display would draw one accelerator for `Archive` and `Archive thread`.
        #expect(MenuPlan.accelerator(from: "A") == "⇧A")
        #expect(MenuPlan.accelerator(from: "a") == "A")
        #expect(MenuPlan.accelerator(from: "cmd+A") == "⇧⌘A")
        #expect(MenuPlan.accelerator(from: "shift+A") == "⇧A")
    }

    @Test func aPunctuationKeyIsDrawnAsItsCharacter() {
        // The resolver spells punctuation by GDK name; a menu printing it would show `⌘COMMA`.
        #expect(MenuPlan.accelerator(from: "cmd+comma") == "⌘,")
        #expect(MenuPlan.accelerator(from: "question") == "?")
        #expect(MenuPlan.accelerator(from: "slash") == "/")
    }

    @Test func aKeypadKeyIsDrawnAsItsCharacter() {
        #expect(MenuPlan.accelerator(from: "cmd+KP_Add") == "⌘+")
        #expect(MenuPlan.accelerator(from: "cmd+KP_Subtract") == "⌘-")
        #expect(MenuPlan.accelerator(from: "cmd+KP_0") == "⌘0")
        #expect(MenuPlan.accelerator(from: "KP_Enter") == "⌅")
    }

    @Test func noAcceleratorLeaksAKeyName() {
        // Over the whole registry: a key name reaching a menu renders as capitals (COMMA, PAGE_UP),
        // which no real accelerator has.
        for spec in PostioRegistry.commands {
            let binding = spec.defaultBinding.replacingOccurrences(of: "mod+", with: "cmd+")
            guard let drawn = MenuPlan.accelerator(from: binding) else { continue }
            let letters = drawn.filter { $0.isLetter }
            let leak = "`\(spec.id)` draws `\(drawn)`, a key name rather than a key"
            #expect(letters.count <= 1, Comment(rawValue: leak))
        }
    }

    @Test func twoCommandsThatBindDifferentKeysDrawDifferentAccelerators() {
        // `a` and `A` are different keys and must not draw the same string; checked over the whole
        // registry.
        var drawnBy: [String: String] = [:]
        for spec in PostioRegistry.commands {
            let binding = spec.defaultBinding.replacingOccurrences(of: "mod+", with: "cmd+")
            guard let drawn = MenuPlan.accelerator(from: binding) else { continue }
            if let already = drawnBy[drawn], already != spec.defaultBinding {
                let clash = "`\(already)` and `\(spec.defaultBinding)` both draw `\(drawn)`"
                Issue.record(Comment(rawValue: clash))
            }
            drawnBy[drawn] = spec.defaultBinding
        }
    }

    @Test func aSequenceIsDrawnAsNoAcceleratorAtAll() {
        // `g g` cannot be a key equivalent, and showing `G` for a command `G` does not run is worse
        // than nothing.
        #expect(MenuPlan.accelerator(from: "g g") == nil)
        let bar = MenuPlan.build(bindings: defaults)
        let first = bar.flatMap(\.items).first { $0.command == "first_message" }
        #expect(first != nil, "first_message never reached a menu")
        #expect(first?.shortcut == nil)
    }
}

@MainActor
@Suite struct ApplicationMenuTests {
    @Test func settingsIsPlannedIntoTheApplicationMenu() {
        // The placement comes from `postio_core::menu`, one table's answer rather than a hand-kept
        // list (#1158).
        let plan = MenuPlan.build(bindings: { _ in ["⌘,"] })
        let app = plan.first { $0.section == .app }
        #expect(app != nil, "no application menu was planned")
        #expect(app?.items.contains { $0.command == "settings" } == true)
        #expect(app?.items.contains { $0.command == "edit_config" } == true)
    }

    @Test func nothingInTheApplicationMenuIsAlsoInEdit() {
        // A fold that adds rather than moves would leave Settings in two menus.
        let plan = MenuPlan.build(bindings: { _ in [] })
        let app = Set(plan.first { $0.section == .app }?.items.compactMap(\.command) ?? [])
        let edit = Set(plan.first { $0.section == .edit }?.items.compactMap(\.command) ?? [])
        #expect(app.isDisjoint(with: edit), "\(app.intersection(edit)) is in both menus")
    }
}

@MainActor
@Suite struct FocusMenuBarTests {
    final class Keys {
        var overrides: [String: [String]] = [:]
        func bindings(_ command: String) -> [String] {
            if let rebound = overrides[command] { return rebound }
            guard let spec = PostioRegistry.commands.first(where: { $0.id == command }) else {
                return []
            }
            return ([spec.defaultBinding] + spec.alternateBindings)
                .map { $0.replacingOccurrences(of: "mod+", with: "cmd+") }
        }
    }

    private func items(_ bar: [MenuPlan.Menu]) -> [MenuPlan.Item] { bar.flatMap(\.items) }

    @Test func everyCommandFocusOffersHereHasAnItemWithTheKeyInForce() {
        let keys = Keys()
        keys.overrides["archive"] = ["ctrl+shift+e"]
        let bar = MenuPlan.bar(bindings: keys.bindings)
        let offered = PostioRegistry.commands.filter { $0.menu != nil }
        #expect(!offered.isEmpty)
        for spec in offered {
            let found = items(bar).filter { $0.command == spec.id }
            #expect(found.count == 1, "`\(spec.id)` is on the bar \(found.count) times")
            #expect(
                found.first?.shortcut == MenuPlan.accelerator(among: keys.bindings(spec.id)),
                "`\(spec.id)` shows \(found.first?.shortcut ?? "nothing")")
        }
        #expect(items(bar).first { $0.command == "archive" }?.shortcut == "⌃⇧E")
    }

    @Test func nothingIsOnTheBarThatFocusDoesNotOfferHere() {
        let offered = Set(PostioRegistry.commands.filter { $0.menu != nil }.map(\.id))
        for item in items(MenuPlan.bar(bindings: Keys().bindings)) {
            guard let command = item.command else { continue }
            #expect(offered.contains(command), "`\(command)` is on the bar and not offered here")
        }
    }

    @Test func theApplicationMenuHasSettingsAndQuitOnce() {
        let bar = MenuPlan.bar(bindings: Keys().bindings)
        let app = bar.first { $0.section == .app }
        let titles = app?.items.map(\.title) ?? []
        #expect(titles.first == "About Postio")
        #expect(app?.items.first { $0.command == "settings" }?.shortcut == "⌘,")
        #expect(titles.contains("Hide Postio"))
        let quits = app?.items.filter { $0.title.hasPrefix("Quit") } ?? []
        #expect(quits.count == 1, "one Quit, not AppKit's and the registry's: \(titles)")
        #expect(quits.first?.command == "quit")
        #expect(quits.first?.shortcut == "⌘Q")
        #expect(titles.last == quits.first?.title, "Quit ends the menu")
    }

    @Test func fileHasNewMessageAndClose() {
        let file = MenuPlan.bar(bindings: Keys().bindings).first { $0.section == .file }
        let new = file?.items.first { $0.command == "compose" }
        #expect(new?.title == "New Message")
        #expect(new?.shortcut == "⌘N")
        let close = file?.items.last
        #expect(close?.role == .close)
        #expect(close?.title == "Close")
        #expect(close?.shortcut == "⌘W")
    }

    @Test func editStartsWithTheEnginesUndoAndTheTextItems() {
        let edit = MenuPlan.bar(bindings: Keys().bindings).first { $0.section == .edit }
        let roles = edit?.items.prefix(6).map(\.role)
        #expect(roles == [.undo, .redo, .cut, .copy, .paste, .selectAll])
        let undo = edit?.items.first
        #expect(undo?.command == "undo", "Edit › Undo is the registry's undo")
        #expect(undo?.shortcut == "⌘Z")
        #expect(edit?.items.filter { $0.role == .undo }.count == 1, "one Undo")
    }

    @Test func windowHasMinimiseAndZoom() {
        let bar = MenuPlan.bar(bindings: Keys().bindings)
        let window = bar.first { $0.title == "Window" }
        #expect(window?.section == nil, "Window is AppKit's, not a registry section")
        #expect(window?.items.map(\.role) == [.minimize, .zoom])
        #expect(bar.last?.title == "Help", "Help stays last, after Window")
    }

    @Test func afterKeymapChangedThePlanIsRebuiltWithTheNewKey() {
        let keys = Keys()
        let plan = MenuBarPlan(bindings: keys.bindings)
        let archive = { plan.menus.flatMap(\.items).first { $0.command == "archive" }?.shortcut }
        let before = archive()
        keys.overrides["archive"] = ["cmd+e"]
        #expect(!plan.apply(.focusListToTop), "another event rebuilds nothing")
        #expect(archive() == before)
        #expect(plan.apply(.keymapChanged))
        #expect(archive() == "⌘E")
    }
}
