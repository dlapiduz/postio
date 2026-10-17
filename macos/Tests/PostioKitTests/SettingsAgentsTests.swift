import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Agent access in Settings (#1832): the pane shows what `[mcp]` shares and
/// revoking goes back through the boundary, leaving the rest of the file.
@MainActor
@Suite struct SettingsAgentsTests {
    private static let shared = """
        # mine
        [ui]
        theme = "dark"

        [mcp.accounts."ada@example.com"]
        mailboxes = ["INBOX", "Receipts"]

        [mcp.accounts."grace@example.com"]
        mailboxes = ["*"]
        actions = ["archive", "label"]

        """

    private func store(_ tag: String, _ text: String) throws -> (SettingsStore, String) {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("postio-agents-\(tag)-\(ProcessInfo.processInfo.processIdentifier)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let path = dir.appendingPathComponent("config.toml").path
        try text.write(toFile: path, atomically: true, encoding: .utf8)
        return (SettingsStore(path: path), path)
    }

    @Test func theNavListsAgentsAboveTheRawFile() {
        let labels = settingsSections().map(\.label)
        #expect(labels.suffix(2) == ["Agents", "Config file"], "\(labels)")
        #expect(settingsSections().first { $0.label == "Agents" }?.key == "mcp")
    }

    @Test func theSharedAccountsAreListedWithTheirFoldersAndAbilities() throws {
        let (store, _) = try store("list", Self.shared)
        let rows = try #require(store.agentSharing)
        #expect(rows.map(\.address) == ["ada@example.com", "grace@example.com"])
        #expect(rows[0].folders == "INBOX, Receipts")
        #expect(rows[0].abilities == "Read and draft only")
        #expect(rows[1].folders == "All folders")
        #expect(rows[1].abilities == "Also archive and label")
    }

    @Test func nothingIsSharedUntilTheFileSaysSo() throws {
        let (store, path) = try store("none", "")
        #expect(try #require(store.agentSharing).isEmpty)
        #expect(settingsAgentWords().nothingShared.contains("No agent"))
        #expect(try String(contentsOfFile: path, encoding: .utf8) == "", "looking wrote a file")
    }

    @Test func revokingOneAccountLeavesTheOtherAndTheRestOfTheFile() throws {
        let (store, path) = try store("one", Self.shared)
        store.revokeAgent("ada@example.com")

        #expect(try #require(store.agentSharing).map(\.address) == ["grace@example.com"])
        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("# mine") && written.contains("theme = \"dark\""), "\(written)")
        #expect(!written.contains("ada@example.com"), "\(written)")
    }

    @Test func revokingEverythingSharesNothing() throws {
        let (store, path) = try store("all", Self.shared)
        store.revokeAllAgents()

        #expect(try #require(store.agentSharing).isEmpty)
        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(!written.contains("mcp") && written.contains("theme = \"dark\""), "\(written)")
    }

    @Test func revokingReadsTheFileAgainSoAnEditFromElsewhereSurvives() throws {
        let (store, path) = try store("fresh", Self.shared)
        // The person edits the file by hand while the window is open.
        try (Self.shared + "\n[sync]\nidle = true\n").write(
            toFile: path, atomically: true, encoding: .utf8)
        store.revokeAgent("grace@example.com")

        let written = try String(contentsOfFile: path, encoding: .utf8)
        #expect(written.contains("idle = true"), "the hand edit was overwritten: \(written)")
    }

    @Test func aFileThatWillNotParseDisablesThePaneAndRevokeWritesNothing() throws {
        let broken = "[mcp\n"
        let (store, path) = try store("broken", broken)
        #expect(store.agentSharing == nil)
        store.revokeAgent("ada@example.com")
        #expect(try String(contentsOfFile: path, encoding: .utf8) == broken)
        #expect(store.failure != nil, "a refusal names its reason")
    }

    @Test func theEntryPointIsBesideTheAppsOwnExecutable() {
        let app = URL(fileURLWithPath: "/Applications/Postio.app/Contents/MacOS/Postio")
        #expect(
            AgentEntryPoint.path(executable: app)
                == "/Applications/Postio.app/Contents/MacOS/postio-mcp")
        #expect(
            AgentEntryPoint.path(executable: URL(fileURLWithPath: "/tmp/.build/debug/Postio")) == nil,
            "a bare `swift run` has no bundle to point into")
        #expect(AgentEntryPoint.path(executable: nil) == nil)
    }

    @Test func revokingPromisesTheNextCallAndNotTheOneRunning() {
        let note = settingsAgentWords().revokeNote
        #expect(note.contains("next call") && note.contains("already running"))
    }
}
