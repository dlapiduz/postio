import PostioFFI
import Testing

@testable import PostioKit

@Suite struct KeyboardLayersTests {
    private func spec(_ id: String) -> CommandSpecFfi? {
        PostioRegistry.commands.first { $0.id == id }
    }

    private func bindings(_ id: String) -> [String] {
        guard let spec = spec(id) else { return [] }
        return ([spec.defaultBinding] + spec.alternateBindings)
            .map { $0.replacingOccurrences(of: "mod+", with: "cmd+") }
    }

    private let canvas: [(command: String, chord: String, mnemonic: String)] = [
        ("reply", "⌘R", "r"),
        ("reply_all", "⇧⌘R", "a"),
        ("forward", "⇧⌘F", "f"),
        ("archive", "⇧⌘A", "e"),
        ("compose", "⌘N", "c"),
        ("search", "⌥⌘F", "/"),
        ("next_in_thread", "⌥↓", "]"),
        ("prev_in_thread", "⌥↑", "["),
        ("next_message", "↓", "j"),
        ("prev_message", "↑", "k"),
        ("expand_all", "⇧⌘E", ";"),
        ("undo", "⌘Z", "z"),
        ("settings", "⌘,", ""),
    ]

    @Test func everyCommandInTheCanvasTableKeepsItsMnemonic() {
        for row in canvas where !row.mnemonic.isEmpty {
            #expect(
                spec(row.command)?.defaultBinding == row.mnemonic,
                "\(row.command) should still answer to \(row.mnemonic)"
            )
        }
    }

    @Test func everyCommandInTheCanvasTableHasTheChordTheCanvasDraws() {
        for row in canvas where !row.chord.isEmpty {
            let drawn = MenuPlan.accelerator(among: bindings(row.command))
            #expect(drawn == row.chord, "\(row.command) draws \(drawn ?? "nothing")")
        }
    }

    @Test func everySurfaceThatNamesAKeyNamesTheSameOne() {
        // The conversation pane drew `E` beside Reply while the Message menu drew `⌘R`; one rule
        // must give every surface the same key.
        for row in canvas where !row.chord.isEmpty && !row.mnemonic.isEmpty {
            let drawn = MenuPlan.accelerator(among: bindings(row.command))
            let fromThePrimaryAlone = MenuPlan.accelerator(from: row.mnemonic)

            #expect(drawn == row.chord)
            #expect(
                drawn != fromThePrimaryAlone,
                "\(row.command) draws its mnemonic where the canvas draws a chord"
            )
        }
    }

    @Test func aCommandWithNoChordDrawsItsMnemonicRatherThanNothing() {
        // A menu item with no accelerator is worse than one naming the key that works.
        #expect(MenuPlan.accelerator(among: bindings("mute_thread")) == "M")
    }
}
