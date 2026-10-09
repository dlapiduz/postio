import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The settings window's model: what it shows, and what reaches the file.
///
/// Everything here is about the boundary's contract holding on this side of
/// it. The Rust suite already proves the `patch_*` edits preserve the rest of
/// the file; what these assert is that the window actually goes through it
/// rather than around it, which is the mistake a frontend makes once and
/// nobody notices until a config file comes back reordered.
@MainActor
@Suite struct SettingsStoreTests {
    private func tempPath(_ tag: String) -> String {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("postio-settings-\(tag)-\(ProcessInfo.processInfo.processIdentifier)")
        try? FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        return dir.appendingPathComponent("config.toml").path
    }

    @Test func aFirstRunOpensOnDefaultsAndHasNotWrittenAnything() {
        // ADR 0029's Empty state. Creating the file just because the window
        // opened would put a config on disk for someone who only looked.
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

    @Test func theFooterFollowsWhateverIsInTheFile() throws {
        let path = tempPath("footer")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        #expect(!store.status.valid)
        #expect(store.status.line == 2)
        #expect(store.status.statusLine.contains("parsed in"))
    }

    @Test func aFileThatWillNotParseLeavesThePaneWithNothingToShow() throws {
        // The pane disables its controls rather than drawing plausible
        // settings that are not the user's — saving those would erase the file
        // they were trying to fix.
        let path = tempPath("broken")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        #expect(store.filtering == nil)
        #expect(store.composing == nil)
    }

    @Test func theNavIsFocusSEightSectionsUnderTheSameTwoHeadings() {
        // The nav is the shared model's, not a list this frontend keeps
        // beside it -- GTK's Focus window shows these eight in this order,
        // and two navs that drift are two different applications. Focus has
        // Filtering and no Appearance (specs/009-focus-macos T131).
        let store = SettingsStore(path: tempPath("nav"))
        #expect(store.sections.map(\.label) == [
            "Accounts", "Filtering", "Saved searches", "Composing",
            "Keyboard", "Sync & storage", "Privacy", "Config file",
        ])
        #expect(
            store.sections(in: .mail).map(\.label)
                == ["Accounts", "Filtering", "Saved searches", "Composing"])
        #expect(store.selected == "accounts", "the window opens on the nav's first pane")
    }

    @Test func thePaneNamesTheTableItWrites() {
        // The footer reads `[focus] in config.toml · applied live`, and the
        // table comes from the section rather than a string kept next to the
        // view.
        let store = SettingsStore(path: tempPath("table"))
        store.selected = "focus"
        #expect(store.current?.table == "[focus]")
        #expect(store.footer.hasPrefix("[focus] in config.toml · applied live"))
    }

    @Test func theFilteringPaneSaysWhatTheFileSaysAndTodaySCount() throws {
        // GTK's page, word for word: the switch, the sentence under it, and
        // the count only once it is known.
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
        // Canvas 3d: the footer is the only line there is, so when something
        // is wrong it has to say that rather than keep announcing which table
        // it would have written.
        let path = tempPath("footer-takeover")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.selected = "focus"
        #expect(store.footer.contains("line 2"), "\(store.footer)")
        #expect(!store.footer.contains("applied live"))
    }

    @Test func anEditMadeInAnEditorWhileTheWindowIsOpenIsNotDestroyedByTheNextClick() throws {
        // Found by using the running app, not by these tests: the window had
        // been open since before the file existed, so its cached copy was two
        // edits behind, and one click on a segmented control wrote that copy
        // back over everything $EDITOR had added.
        //
        // `config.toml` is the store and a file people edit by hand. Anything
        // that patches a *remembered* version of it is a second writer racing
        // the first, and the loser is whichever one the user typed into.
        let path = tempPath("external-edit")
        try "[focus]\nfiltering = true\n".write(toFile: path, atomically: true, encoding: .utf8)
        let store = SettingsStore(path: path)

        // Somebody runs ⌘E and adds to the file while the window sits open.
        // The comment sits above `[sync]`, outside the table the switch
        // edits: everything outside the patched key is promised, and that is
        // what this asserts.
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
        // The footer is the only thing telling the user their change took —
        // canvas 3f put it where OK and Cancel would be — so it has to
        // describe the file as it is now, not as it was when the window opened.
        let path = tempPath("restat")
        try "[focus]\nfiltering = true\n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.applyFiltering(on: false)

        #expect(store.status.valid)
        #expect(store.filtering?.on == false)
    }

    @Test func theAccountsPaneDoesNotClaimToWriteConfigToml() {
        // It said `[accounts] in config.toml` about accounts that have lived
        // in the encrypted store since #470 — a footer pointing somebody at a
        // file that does not describe their account. Seen by opening the pane.
        let store = SettingsStore(path: tempPath("accounts-footer"))
        store.selected = "accounts"

        #expect(!store.footer.contains("[accounts]"), "\(store.footer)")
        #expect(store.footer.contains("encrypted store"), "\(store.footer)")
    }
}

/// Every one of canvas 3f's panes is drawn (#1156).
///
/// The nav comes from `postio_ui::settings`, so a section the frontend has no
/// pane for still appears in the sidebar and opens an empty apology. That was
/// five of the eight for a long time; this is what stops it silently becoming
/// five again when a section is added to the core.
@MainActor
@Suite struct EveryPaneIsDrawnTests {
    @Test func noSectionFallsThroughToTheUnbuiltPane() {
        // The keys `SettingsPaneView` switches on, kept beside the switch
        // rather than inferred: the point is to fail when the core grows a
        // section and this frontend has not caught up.
        let drawn: Set<String> = ["accounts", "focus", "saved_searches", "compose", "keys", "sync", "privacy", ""]

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
