import PostioFFI
import Testing

@testable import PostioKit

/// The key map (`?`, screen 20; specs/009-focus-macos T106): the
/// controller opens it as `FocusOpenKeyMap`, carrying the sheet
/// `postio_ui::keymap_sheet` groups for this platform, and closes it with
/// `FocusCloseSurface(.keyMap)`. The Mac lays the groups out in the columns
/// the sheet names and spells every key as the menus do.
@MainActor
@Suite struct KeyMapSheetTests {
    static let sheet = KeyMapSheetFfi(
        title: "Keys",
        subtitle: "Single keys act on the focused row, or on the selection if there is one.",
        closeKeys: ["?", "Escape"],
        closeWord: "close",
        closeOr: "or",
        groups: [
            KeyMapGroupFfi(
                title: "Move and select",
                rows: [
                    KeyMapRowFfi(command: "next_message", title: "Next message", keys: ["j", "Down"]),
                    KeyMapRowFfi(command: "first_message", title: "Top / bottom", keys: ["g g", "G"]),
                ]),
            KeyMapGroupFfi(
                title: "Go and find",
                rows: [
                    KeyMapRowFfi(command: "command_palette", title: "Command bar", keys: ["cmd+k"]),
                    KeyMapRowFfi(command: "go_to_inbox", title: "Inbox", keys: ["g i"]),
                ]),
            KeyMapGroupFfi(
                title: "Act (row or selection)",
                rows: [KeyMapRowFfi(command: "undo", title: "Undo", keys: ["cmd+z"])]),
            KeyMapGroupFfi(
                title: "Unbound",
                rows: [KeyMapRowFfi(command: "refresh", title: "Refresh", keys: [])]),
        ],
        columns: [[0], [2], [1, 3], [7]],
        rebindFooter: "Rebind anything in ~/Library/Application Support/Postio/config.toml under [keys]",
        mouseFooter: "The mouse works everywhere: every key has a visible button.")

    // MARK: the words, laid out

    @Test func theGroupsStandInTheColumnsTheSheetNames() {
        let words = KeyMapSheetWords(Self.sheet)
        #expect(words.columns.map { $0.map(\.title) } == [
            ["Move and select"], ["Act (row or selection)"], ["Go and find", "Unbound"], [],
        ])
    }

    @Test func aGroupIsKeptWholeInItsOrder() {
        let words = KeyMapSheetWords(Self.sheet)
        #expect(words.columns[0].first?.rows.map(\.title) == ["Next message", "Top / bottom"])
    }

    @Test func everyKeyIsSpelledAsTheMenusSpellItInOneCap() {
        let words = KeyMapSheetWords(Self.sheet)
        let caps = words.columns.flatMap { $0 }.flatMap(\.rows).map(\.cap)
        #expect(caps == ["j ↓", "g g G", "⌘Z", "⌘K", "g i", nil])
    }

    @Test func theCloseKeysAreCapsWithTheControllersWords() {
        let words = KeyMapSheetWords(Self.sheet)
        #expect(words.closeCaps == ["?", "⎋"])
        #expect(words.closeOr == "or")
        #expect(words.closeWord == "close")
    }

    @Test func theFooterNamesTheConfigFileAndItsTable() {
        // C3: `[keys]` in `config.toml`, not a `keys.toml`, whatever the
        // pack's footer says.
        let words = KeyMapSheetWords(Self.sheet)
        #expect(words.rebind.contains("~/Library/Application Support/Postio/config.toml"))
        #expect(words.rebind.contains("[keys]"))
        #expect(!words.rebind.contains("keys.toml"))
        #expect(words.mouse == Self.sheet.mouseFooter)
        #expect(words.title == "Keys")
        #expect(words.subtitle == Self.sheet.subtitle)
    }

    // MARK: open and closed, as the controller says

    @Test func theControllerOpensItAndClosesIt() {
        let model = KeyMapModel()
        #expect(model.words == nil)
        #expect(model.apply(.focusOpenKeyMap(sheet: Self.sheet)) == .open)
        #expect(model.words == KeyMapSheetWords(Self.sheet))
        #expect(model.apply(.focusCloseSurface(kind: .picker)) == nil, "another surface's close")
        #expect(model.isOpen)
        #expect(model.apply(.focusCloseSurface(kind: .keyMap)) == .close)
        #expect(!model.isOpen)
    }

    @Test func aRebindRedrawsTheOpenSheetOnly() {
        let model = KeyMapModel()
        var rebound = Self.sheet
        rebound.groups[2].rows[0].keys = ["cmd+shift+z"]
        model.refresh(rebound)
        #expect(!model.isOpen, "a closed key map is not opened by a rebind")
        _ = model.apply(.focusOpenKeyMap(sheet: Self.sheet))
        model.refresh(rebound)
        #expect(model.words?.columns[1].first?.rows.first?.cap == "⇧⌘Z")
    }

    @Test func aClickOutsideIsTheToolkitsCloseSaidOnce() {
        let model = KeyMapModel()
        _ = model.apply(.focusOpenKeyMap(sheet: Self.sheet))
        #expect(model.closedByToolkit())
        #expect(!model.isOpen)
        #expect(!model.closedByToolkit(), "said once")
    }
}
