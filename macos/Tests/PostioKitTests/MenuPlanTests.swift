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
    private func defaults(_ command: String) -> String? {
        PostioRegistry.commands.first { $0.id == command }?.defaultBinding
            .replacingOccurrences(of: "mod+", with: "cmd+")
    }

    @Test func theMenuBarIsBuiltFromTheRegistry() {
        let bar = MenuPlan.build(binding: defaults)
        #expect(!bar.isEmpty, "no menus at all")

        // Every item names a command the registry knows. The failure this
        // guards is a Swift-side list drifting from the Rust one -- which
        // cannot happen while the rows come from `commands()`, and this is
        // what would notice if somebody made it possible again.
        let known = Set(PostioRegistry.commands.map(\.id))
        for menu in bar {
            for item in menu.items {
                #expect(known.contains(item.command), "\(item.command) is not in the registry")
            }
        }
    }

    @Test func aCommandGoesUnderTheMenuTheCoreChose() {
        let bar = MenuPlan.build(binding: defaults)
        let message = bar.first { $0.title == "Message" }
        #expect(message?.items.contains { $0.command == "archive" } == true)
        // ...and not under some other one, or the grouping is not a grouping.
        let file = bar.first { $0.title == "File" }
        #expect(file?.items.contains { $0.command == "archive" } == false)
    }

    @Test func noMenuIsDrawnEmpty() {
        // A section with nothing under it is a pane that opens onto nothing,
        // which reads as a broken application rather than an empty section.
        for menu in MenuPlan.build(binding: { _ in nil }) {
            #expect(!menu.items.isEmpty, "\(menu.title) is empty")
        }
    }

    @Test func theAcceleratorIsWhatIsBoundNotWhatTheRegistryDefaultsTo() {
        // The whole reason `binding(for:)` exists. A menu drawing the default
        // for a command somebody rebound is confidently wrong, which is worse
        // than showing no key at all.
        let bar = MenuPlan.build(binding: { $0 == "archive" ? "ctrl+shift+e" : nil })
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
        let bar = MenuPlan.build(binding: defaults)
        let first = bar.flatMap(\.items).first { $0.command == "first_message" }
        #expect(first != nil, "first_message never reached a menu")
        #expect(first?.shortcut == nil)
    }
}
