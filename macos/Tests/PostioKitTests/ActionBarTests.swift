import PostioFFI
import Testing

@testable import PostioKit

/// The action bar's words (specs/009-focus-macos T052, screen 01): the
/// count the controller composed, the verbs in the order the screen draws
/// them with the user's keys, and the selection's own keys on the right.
struct ActionBarTests {
    static let keys: [String: String] = [
        "archive": "a", "snooze": "s", "toggle_read": "r", "digest_rule": "d",
        "capture_task": "t", "add_label": "l", "move": "m",
        "toggle_selection": "x", "extend_selection_down": "J", "extend_selection_up": "K",
        "back": "Escape",
    ]

    static func words(summary: String? = "3 selected", selecting: Bool = true, vault: Bool = false)
        -> ActionBarWords?
    {
        ActionBarWords(summary: summary, hasSelection: selecting, vault: vault) { keys[$0] }
    }

    @Test func thereIsNoBarWithNothingSelected() {
        #expect(Self.words(summary: nil, selecting: false) == nil)
    }

    @Test func theCountIsTheControllersWords() {
        #expect(Self.words()?.summary == "3 selected")
        #expect(Self.words(summary: "All 312 selected")?.summary == "All 312 selected")
    }

    @Test func theVerbsAreScreenOnesInItsOrder() {
        let labels = Self.words()?.actions.map(\.label)
        #expect(labels == ["Archive", "Snooze", "Mark read", "Digest these\u{2026}", "Label", "Move"])
        let commands = Self.words()?.actions.map(\.command)
        #expect(commands == ["archive", "snooze", "toggle_read", "digest_rule", "add_label", "move"])
    }

    @Test func taskIsOfferedOnlyWithAVault() {
        // C9: capture is gated on a vault; a Task button that could only
        // fail is worse than none.
        #expect(Self.words(vault: false)?.actions.contains { $0.command == "capture_task" } == false)
        let labels = Self.words(vault: true)?.actions.map(\.label)
        #expect(labels == [
            "Archive", "Snooze", "Mark read", "Digest these\u{2026}", "Task", "Label", "Move",
        ])
    }

    @Test func eachVerbsKeycapIsTheUsersBinding() {
        let caps = Self.words()?.actions.map(\.cap)
        #expect(caps == ["a", "s", "r", "d", "l", "m"])
        let rebound = ActionBarWords(summary: "1 selected", hasSelection: true, vault: false) {
            $0 == "archive" ? "e" : nil
        }
        #expect(rebound?.actions.first?.cap == "e")
        #expect(rebound?.actions.last?.cap == nil, "no binding, no cap")
    }

    @Test func archiveIsTheDefault() {
        // Screen 01 draws Archive on the raised fill: the one a selection is
        // usually for.
        #expect(Self.words()?.actions.first { $0.isDefault }?.command == "archive")
        #expect(Self.words()?.actions.filter(\.isDefault).count == 1)
    }

    @Test func theHintsAreTheSelectionsOwnKeys() {
        let hints = Self.words()?.hints
        #expect(hints?.map(\.label) == ["toggle", "extend", "clear"])
        #expect(hints?.first?.caps == ["x"])
        #expect(hints?[1].caps == ["J", "K"])
        #expect(hints?.last?.caps == [KeyCapSpelling.cap("Escape")!])
    }

    @Test func aHintWithNoKeyIsLeftOut() {
        let words = ActionBarWords(summary: "1 selected", hasSelection: true, vault: false) {
            $0 == "toggle_selection" ? "x" : nil
        }
        #expect(words?.hints.map(\.label) == ["toggle"])
    }
}
