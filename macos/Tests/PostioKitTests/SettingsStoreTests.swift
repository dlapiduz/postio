import Foundation
import PostioFFI
import Testing

@testable import PostioKit

@MainActor
@Suite struct SettingsStoreTests {
    private func tempPath(_ tag: String) -> String {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("postio-settings-\(tag)-\(ProcessInfo.processInfo.processIdentifier)")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("config.toml").path
    }

    @Test func aFirstRunOpensOnDefaultsAndHasNotWrittenAnything() {
        // ADR 0029's Empty state: creating the file because the window opened would put a config on
        // disk for someone who only looked.
        let path = tempPath("first-run")
        let store = SettingsStore(path: path)

        #expect(store.filtering != nil, "defaults are readable from an empty document")
        #expect(store.composing != nil, "defaults are readable from an empty document")
        #expect(store.status.valid)
        #expect(!FileManager.default.fileExists(atPath: path), "opening the window wrote a file")
    }

    @Test func changingOneSettingLandsInTheFileAndLeavesTheRestAlone() throws {
        let path = tempPath("patch")
        let original = "# mine\n[sync]\nidle = true\n\n[focus]\nmystery = 1\n"
        try original.write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.applyFiltering(on: false)

        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("filtering = false"), "\(written)")
        #expect(written.contains("# mine"), "the comment did not survive: \(written)")
        #expect(written.contains("idle = true"), "[sync] moved: \(written)")
        #expect(written.contains("mystery = 1"), "an unknown key was dropped: \(written)")
    }

    @Test func theUnsubscribeActionDefaultsToArchiveAndOneKeyChangesIt() throws {
        let path = tempPath("after-unsubscribe")
        try "# mine\n[ui]\ndensity = \"compact\"\n".write(
            toFile: path, atomically: true, encoding: .utf8)
        let store = SettingsStore(path: path)
        let control = try #require(store.afterUnsubscribe)
        #expect(control.choices == ["Archive", "Delete"])
        #expect(!control.delete, "archive is the default")

        store.applyAfterUnsubscribe(delete: true)
        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("after_unsubscribe = \"delete\""), "\(written)")
        #expect(written.contains("# mine"), "the comment did not survive: \(written)")
        #expect(store.afterUnsubscribe?.delete == true)
    }

    @Test func theFooterFollowsWhateverIsInTheFile() throws {
        let path = tempPath("footer")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        #expect(!store.status.valid)
        #expect(store.status.line == 2)
        #expect(store.status.statusLine.contains("parsed in"))
    }

    @Test func aFileThatWillNotParseLeavesThePaneWithNothingToShow() throws {
        let path = tempPath("broken")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        #expect(store.filtering == nil)
        #expect(store.composing == nil)
    }

    @Test func theNavIsFocusSTenSectionsUnderTheSameTwoHeadings() {
        let store = SettingsStore(path: tempPath("nav"))
        #expect(store.sections.map(\.label) == [
            "Accounts", "Contacts", "Filtering", "Saved searches", "Composing",
            "Keyboard", "Sync & storage", "Privacy", "Agents", "Config file",
        ])
        #expect(
            store.sections(in: .mail).map(\.label)
                == ["Accounts", "Contacts", "Filtering", "Saved searches", "Composing"])
        #expect(store.selected == "accounts", "the window opens on the nav's first pane")
    }

    @Test func thePaneNamesTheTableItWrites() {
        let store = SettingsStore(path: tempPath("table"))
        store.selected = "focus"
        #expect(store.current?.table == "[focus]")
        #expect(store.footer.hasPrefix("[focus] in config.toml · applied live"))
    }

    @Test func theFilteringPaneSaysWhatTheFileSaysAndTodaySCount() throws {
        let path = tempPath("filtering")
        try "[focus]\nfiltering = true\n".write(toFile: path, atomically: true, encoding: .utf8)
        let store = SettingsStore(path: path)

        let before = try #require(store.filtering)
        #expect(before.on)
        #expect(before.today == nil, "no count is drawn before one is known")

        store.filteredToday = 186
        let after = try #require(store.filtering)
        #expect(after.today?.contains("186") == true, "\(String(describing: after.today))")
    }

    @Test func takingANeverEntryBackRemovesItAndLeavesTheRestAlone() throws {
        let path = tempPath("take-back")
        let original =
            "# mine\n[focus]\nfiltering = true\n\n[focus.filter]\nnever = [\"ada@example.com\"]\n"
        try original.write(toFile: path, atomically: true, encoding: .utf8)
        let store = SettingsStore(path: path)

        let entry = try #require(store.filtering?.never.first)
        store.takeBack(entry.undo)

        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(!written.contains("ada@example.com"), "\(written)")
        #expect(written.contains("# mine"), "the comment did not survive: \(written)")
        #expect(store.filtering?.never.isEmpty == true)
    }

    @Test func aBrokenFileTakesOverTheFooterFromTheTableName() throws {
        let path = tempPath("footer-takeover")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.selected = "focus"
        #expect(store.footer.contains("line 2"), "\(store.footer)")
        #expect(!store.footer.contains("applied live"))
    }

    @Test func anEditMadeInAnEditorWhileTheWindowIsOpenIsNotDestroyedByTheNextClick() throws {
        // Found by using the app: the window was open from before the file existed, and one click
        // wrote its stale copy over everything `$EDITOR` had added. `config.toml` is hand-edited,
        // so patching a remembered copy is a second writer racing the first.
        let path = tempPath("external-edit")
        try "[focus]\nfiltering = true\n".write(toFile: path, atomically: true, encoding: .utf8)
        let store = SettingsStore(path: path)

        try("[focus]\nfiltering = true\nsome_future_key = 42\n\n# hand-written\n[sync]\nidle = true\n")
            .write(toFile: path, atomically: true, encoding: .utf8)

        store.applyFiltering(on: false)

        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("filtering = false"), "the click did not land: \(written)")
        #expect(written.contains("# hand-written"), "a comment on another table was destroyed: \(written)")
        #expect(written.contains("some_future_key = 42"), "an unknown key was dropped: \(written)")
        #expect(written.contains("[sync]"), "a whole table was destroyed: \(written)")
    }

    @Test func aSaveIsReadBackSoTheFooterCannotGoStale() throws {
        let path = tempPath("restat")
        try "[focus]\nfiltering = true\n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.applyFiltering(on: false)

        #expect(store.status.valid)
        #expect(store.filtering?.on == false)
    }

    @Test func theAccountsPaneDoesNotClaimToWriteConfigToml() {
        // The footer said `[accounts] in config.toml` about accounts that live in the encrypted
        // store since #470.
        let store = SettingsStore(path: tempPath("accounts-footer"))
        store.selected = "accounts"

        #expect(!store.footer.contains("[accounts]"), "\(store.footer)")
        #expect(store.footer.contains("encrypted store"), "\(store.footer)")
    }
}

@MainActor
@Suite struct EveryPaneIsDrawnTests {
    @Test func noSectionFallsThroughToTheUnbuiltPane() {
        // Kept beside the switch so this fails when the core grows a section the frontend has not
        // caught up with.
        let drawn: Set<String> = ["accounts", "focus", "saved_searches", "compose", "keys", "sync", "privacy", "mcp", "contacts", ""]

        let sections = Set(settingsSections().map(\.key))

        #expect(
            sections.subtracting(drawn).isEmpty,
            "sections with no pane: \(sections.subtracting(drawn).sorted())"
        )
        #expect(
            drawn.subtracting(sections).isEmpty,
            "panes for sections that no longer exist: \(drawn.subtracting(sections).sorted())"
        )
    }
}
