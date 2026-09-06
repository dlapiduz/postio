import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The settings window's model: what it shows, and what reaches the file.
///
/// Everything here is about the boundary's contract holding on this side of
/// it. The Rust suite already proves `patch_appearance` preserves the rest of
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

        #expect(store.appearance != nil, "defaults are readable from an empty document")
        #expect(store.status.valid)
        #expect(!FileManager.default.fileExists(atPath: path), "opening the window wrote a file")
    }

    @Test func changingOneSettingLandsInTheFileAndLeavesTheRestAlone() throws {
        let path = tempPath("patch")
        let original = "# mine\n[sync]\nidle = true\n\n[ui]\ntheme = \"dark\"\nmystery = 1\n"
        try original.write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.apply { $0.density = .compact }

        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("density = \"compact\""), "\(written)")
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
        #expect(store.appearance == nil)
    }

    @Test func theNavIsTheSameEightSectionsUnderTheSameTwoHeadings() {
        // The nav is the shared model's, not a list this frontend keeps
        // beside it — the GTK window shows these eight in this order, and two
        // navs that drift are two different applications.
        let store = SettingsStore(path: tempPath("nav"))
        #expect(store.sections.map(\.label) == [
            "Accounts", "Filters", "Composing", "Appearance",
            "Keyboard", "Sync & storage", "Privacy", "Config file",
        ])
        #expect(store.sections(in: .mail).map(\.label) == ["Accounts", "Filters", "Composing"])
        #expect(store.selected == "ui", "the only pane built on macOS so far")
    }

    @Test func thePaneNamesTheTableItWrites() {
        // The footer reads `[ui] in config.toml · applied live`, and the table
        // comes from the section rather than a string kept next to the view.
        let store = SettingsStore(path: tempPath("table"))
        #expect(store.current?.table == "[ui]")
        #expect(store.footer.hasPrefix("[ui] in config.toml · applied live"))
    }

    @Test func aBrokenFileTakesOverTheFooterFromTheTableName() throws {
        // Canvas 3d: the footer is the only line there is, so when something
        // is wrong it has to say that rather than keep announcing which table
        // it would have written.
        let path = tempPath("footer-takeover")
        try "[ui]\ndensity = = \n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
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
        try "[ui]\ntheme = \"dark\"\n".write(toFile: path, atomically: true, encoding: .utf8)
        let store = SettingsStore(path: path)

        // Somebody runs ⌘E and adds to the file while the window sits open.
        // The comment sits above `[sync]` rather than above `[ui]` on purpose:
        // `patch_ui` rewrites its own table wholesale, so a comment attached
        // to `[ui]` is explicitly *not* promised -- `postio_config::ui` calls
        // that "the deliberate half of the promise". Everything outside the
        // patched table is promised, and that is what this asserts.
        try("[ui]\ntheme = \"dark\"\nsome_future_key = 42\n\n# hand-written\n[sync]\nidle = true\n")
            .write(toFile: path, atomically: true, encoding: .utf8)

        store.apply { $0.density = .compact }

        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("density = \"compact\""), "the click did not land: \(written)")
        #expect(written.contains("# hand-written"), "a comment on another table was destroyed: \(written)")
        #expect(written.contains("some_future_key = 42"), "an unknown key was dropped: \(written)")
        #expect(written.contains("[sync]"), "a whole table was destroyed: \(written)")
    }

    @Test func aSaveIsReadBackSoTheFooterCannotGoStale() throws {
        // The footer is the only thing telling the user their change took —
        // canvas 3f put it where OK and Cancel would be — so it has to
        // describe the file as it is now, not as it was when the window opened.
        let path = tempPath("restat")
        try "[ui]\ntheme = \"dark\"\n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        store.apply { $0.theme = .light }

        #expect(store.status.valid)
        #expect(store.appearance?.theme == .light)
    }
}
