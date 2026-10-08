import PostioFFI
import Testing

@testable import PostioKit

/// The menu bar, decided from the registry.
///
/// #657's acceptance criteria, asserted: every registry command reachable or
/// deliberately excluded, accelerators from what is *in force*, and no list of
/// commands written in Swift.
@Suite struct MenuPlanTests {
    /// A binding lookup that answers the registry's defaults, expanded the
    /// way the boundary expands them on this platform.
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

        // Every item names a command the registry knows. The failure this
        // guards is a Swift-side list drifting from the Rust one -- which
        // cannot happen while the rows come from `commands()`, and this is
        // what would notice if somebody made it possible again.
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
        // ...and not under some other one, or the grouping is not a grouping.
        let file = bar.first { $0.title == "File" }
        #expect(file?.items.contains { $0.command == "archive" } == false)
    }

    @Test func noMenuIsDrawnEmpty() {
        // A section with nothing under it is a pane that opens onto nothing,
        // which reads as a broken application rather than an empty section.
        for menu in MenuPlan.build(bindings: { _ in [] }) {
            #expect(!menu.items.isEmpty, "\(menu.title) is empty")
        }
    }

    @Test func theAcceleratorIsWhatIsBoundNotWhatTheRegistryDefaultsTo() {
        // The whole reason `binding(for:)` exists. A menu drawing the default
        // for a command somebody rebound is confidently wrong, which is worse
        // than showing no key at all.
        let bar = MenuPlan.build(bindings: { $0 == "archive" ? ["ctrl+shift+e"] : [] })
        let archive = bar.flatMap(\.items).first { $0.command == "archive" }
        #expect(archive?.shortcut == "⌃⇧E")
    }

    @Test func modifiersAreDrawnInApplesOrderNotTheBindingsOrder() {
        // ⌃⌥⇧⌘, always, whatever order somebody typed into `[keys]`.
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
        // The resolver folds Shift into the character for a key that types
        // one -- `shift+a` *is* `A` -- so eleven registry defaults are bare
        // capitals. Uppercasing them for display loses the fold, and
        // `Archive` and `Archive thread` then draw the identical accelerator
        // for two different commands. Seen in the Message menu before it was
        // fixed; invisible in every test that existed.
        #expect(MenuPlan.accelerator(from: "A") == "⇧A")
        #expect(MenuPlan.accelerator(from: "a") == "A")
        #expect(MenuPlan.accelerator(from: "cmd+A") == "⇧⌘A")
        // Explicit shift and a folded capital are the same key, drawn once.
        #expect(MenuPlan.accelerator(from: "shift+A") == "⇧A")
    }

    @Test func aPunctuationKeyIsDrawnAsItsCharacter() {
        // The resolver spells punctuation by name -- `mod+comma` is Settings'
        // default -- and the names are GDK's. A menu that printed the name
        // showed `⌘COMMA`, which is not a key anybody can find.
        #expect(MenuPlan.accelerator(from: "cmd+comma") == "⌘,")
        #expect(MenuPlan.accelerator(from: "question") == "?")
        #expect(MenuPlan.accelerator(from: "slash") == "/")
    }

    @Test func noAcceleratorLeaksAKeyName() {
        // The class, over the whole registry rather than the three spellings
        // that happened to be wrong. A key name reaching a menu renders as a
        // run of capitals -- COMMA, RETURN, PAGE_UP -- and a real accelerator
        // never has one: modifiers are glyphs and the key is one character.
        for spec in PostioRegistry.commands {
            let binding = spec.defaultBinding.replacingOccurrences(of: "mod+", with: "cmd+")
            guard let drawn = MenuPlan.accelerator(from: binding) else { continue }
            let letters = drawn.filter { $0.isLetter }
            let leak = "`\(spec.id)` draws `\(drawn)`, a key name rather than a key"
            #expect(letters.count <= 1, Comment(rawValue: leak))
        }
    }

    @Test func twoCommandsThatBindDifferentKeysDrawDifferentAccelerators() {
        // The failure that started this: `Archive` (`a`) and `Archive thread`
        // (`A`) are different keys and drew the same string. Checked over the
        // whole registry, because any pair could do it.
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
        // `g g` cannot be expressed as a key equivalent, and showing `G` for
        // a command that `G` does not run is worse than showing nothing. This
        // is also why the cheat sheet is in the Help menu: it is the only
        // surface that can describe a sequence.
        #expect(MenuPlan.accelerator(from: "g g") == nil)
        let bar = MenuPlan.build(bindings: defaults)
        let first = bar.flatMap(\.items).first { $0.command == "first_message" }
        #expect(first != nil, "first_message never reached a menu")
        #expect(first?.shortcut == nil)
    }
}

/// The application menu, which macOS expects and freedesktop does not have.
@MainActor
@Suite struct ApplicationMenuTests {
    @Test func settingsIsPlannedIntoTheApplicationMenu() {
        // `⌘,` is discoverable there and nowhere else on this platform. The
        // placement comes from `postio_core::menu`, so it is one table's
        // answer rather than a list of command ids kept here by hand — which
        // is what #1158 existed to remove.
        let plan = MenuPlan.build(bindings: { _ in ["⌘,"] })
        let app = plan.first { $0.section == .app }
        #expect(app != nil, "no application menu was planned")
        #expect(app?.items.contains { $0.command == "settings" } == true)
        #expect(app?.items.contains { $0.command == "edit_config" } == true)
    }

    @Test func nothingInTheApplicationMenuIsAlsoInEdit() {
        // The failure this guards is a fold that adds rather than moves,
        // leaving Settings in two menus at once.
        let plan = MenuPlan.build(bindings: { _ in [] })
        let app = Set(plan.first { $0.section == .app }?.items.compactMap(\.command) ?? [])
        let edit = Set(plan.first { $0.section == .edit }?.items.compactMap(\.command) ?? [])
        #expect(app.isDisjoint(with: edit), "\(app.intersection(edit)) is in both menus")
    }
}

/// Focus's menu bar on the Mac (specs/009-focus-macos T104): the registry's
/// menus for what Focus offers here, each item showing the key in force, and
/// the standard items every Mac application has around them.
@MainActor
@Suite struct FocusMenuBarTests {
    /// What `bindingsFor` answers: the registry's defaults as this platform
    /// expands them, with `[keys]`' overrides on top.
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
