import PostioFFI
import Testing

@testable import PostioKit

/// The toolbar the canvas draws, and the two rules it has to keep.
@Suite struct ToolbarPlanTests {
    @Test func everyButtonIsARegistryCommand() {
        // A toolbar button that did its own thing would be a way to archive
        // that undo has never heard of. Held against the registry itself, so
        // a command renamed in Rust fails here rather than drawing a button
        // that quietly does nothing.
        let known = Set(PostioRegistry.commands.map(\.id))
        for item in ToolbarPlan.items {
            #expect(known.contains(item.command), "\(item.command) is not a command")
        }
    }

    @Test func theCanvasOrderIsTheOrder() {
        #expect(ToolbarPlan.items.map(\.command) == ["archive", "flag", "reply", "compose"])
    }

    @Test func anUnlabelledButtonSaysWhatItIsAndWhichKeyDoesIt() {
        // Icons only, so the tooltip is the whole of what a person who does
        // not recognise the glyph has to go on.
        let item = ToolbarPlan.items[0]
        #expect(ToolbarPlan.tooltip(for: item) { _ in "shift+cmd+a" } == "Archive (⇧⌘A)")
    }

    @Test func aCommandWithNoBindingStillHasATooltip() {
        let item = ToolbarPlan.items[0]
        #expect(ToolbarPlan.tooltip(for: item) { _ in nil } == "Archive")
    }

    @Test func aSequenceIsNotDrawnAsAnAccelerator() {
        // `g g` cannot be written as a chord, and half of it would name a key
        // that does something else.
        let item = ToolbarPlan.items[0]
        #expect(ToolbarPlan.tooltip(for: item) { _ in "g g" } == "Archive")
    }
}
