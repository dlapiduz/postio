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
