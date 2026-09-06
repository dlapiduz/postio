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
        var appearance = try #require(store.appearance)
        appearance.density = .compact
        store.apply(appearance)

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

    @Test func theNavIsCanvasOrderAndOpensOnAppearance() {
        let store = SettingsStore(path: tempPath("nav"))
        #expect(store.sections.map(\.title) == [
            "Appearance", "Keyboard", "Accounts", "Sync", "Filters", "Privacy",
        ])
        #expect(store.selected == "ui", "the pane ADR 0029 ships first")
    }

    @Test func aSaveIsReadBackSoTheFooterCannotGoStale() throws {
        // The footer is the only thing telling the user their change took —
        // canvas 3f put it where OK and Cancel would be — so it has to
        // describe the file as it is now, not as it was when the window opened.
        let path = tempPath("restat")
        try "[ui]\ntheme = \"dark\"\n".write(toFile: path, atomically: true, encoding: .utf8)

        let store = SettingsStore(path: path)
        var appearance = try #require(store.appearance)
        appearance.theme = .light
        store.apply(appearance)

        #expect(store.status.valid)
        #expect(store.appearance?.theme == .light)
    }
}
