import PostioFFI
import Testing

@testable import PostioKit

/// Two keyboard layers, both always on (canvas screen 26).
///
/// Every command in the canvas' table carries a `⌘` chord so it can live in
/// the menu bar, which is what a Mac user expects, **and** keeps its
/// single-key mnemonic, which is the application's identity. They are two
/// bindings on one command rather than a mode, so nothing has to be switched
/// and neither can shadow the other.
///
/// Held against the registry, because that is the one table of defaults: a
/// chord dropped in Rust fails here rather than quietly leaving a menu with
/// no accelerator.
@Suite struct KeyboardLayersTests {
    private func spec(_ id: String) -> CommandSpecFfi? {
        PostioRegistry.commands.first { $0.id == id }
    }

    private func bindings(_ id: String) -> [String] {
        guard let spec = spec(id) else { return [] }
        return ([spec.defaultBinding] + spec.alternateBindings)
            .map { $0.replacingOccurrences(of: "mod+", with: "cmd+") }
    }

    /// The canvas' own table: command, chord, mnemonic.
    private let canvas: [(command: String, chord: String, mnemonic: String)] = [
        ("reply", "⌘R", "e"),
        ("reply_all", "⇧⌘R", "E"),
        ("forward", "⇧⌘F", "f"),
        ("archive", "⇧⌘A", "a"),
        ("archive_thread", "", "A"),
        ("compose", "⌘N", "c"),
        ("search", "⌥⌘F", "/"),
        ("next_in_conversation", "⌥↓", "J"),
        ("prev_in_conversation", "⌥↑", "K"),
        ("next_message", "↓", "j"),
        ("prev_message", "↑", "k"),
        ("expand_all", "⇧⌘E", "O"),
        ("undo", "⌘Z", "u"),
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
        // The bug this closes: the conversation pane asked for the *primary*
        // binding and drew `E` beside Reply while the Message menu drew `⌘R`.
        // Both are live, both are correct, and a window showing each in a
        // different place teaches neither.
        for row in canvas where !row.chord.isEmpty && !row.mnemonic.isEmpty {
            // What the rule draws, against what asking for the primary
            // binding alone would have drawn. They must differ, and the first
            // is the one every surface gets.
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
        // Archive-thread is `A` and nothing else — the canvas says so with a
        // dash. A menu item with no accelerator at all is worse than one
        // naming the key that does work.
        #expect(MenuPlan.accelerator(among: bindings("archive_thread")) == "⇧A")
    }
}
