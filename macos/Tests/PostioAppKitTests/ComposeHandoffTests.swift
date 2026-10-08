import Foundation
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// Which editor a draft goes to (#1288): the half that asks this Mac what it
/// has, which needs `NSWorkspace` and so lives with `ComposeHandoff`'s AppKit
/// extension. The model half stays in `ComposeModelTests`.
@MainActor
@Suite struct ComposeHandoffTests {
    @Test func aTerminalEditorIsNotOpenedAndSaysWhy() {
        // `vi` ships with macOS and has no window of its own.
        #expect(ComposeHandoff.found("vi") == .terminalProgram)

        let target = settingsHandoffTarget(configured: "vi", found: .terminalProgram)
        guard case .needsTerminal(_, let advice) = target else {
            Issue.record("expected a terminal program, got \(target)")
            return
        }
        #expect(advice.contains("terminal"))
    }

    @Test func anEditorThatIsNotThereSaysThatRatherThanBlamingTheTerminal() {
        // The distinction GTK's adoption forced (#1297): a name that is not
        // an application is a terminal program on macOS *usually* — and a
        // typo the rest of the time. Nothing on this Mac is called this.
        let typo = "postio-no-such-editor-1297"
        #expect(ComposeHandoff.found(typo) == .nothing)

        let target = settingsHandoffTarget(configured: typo, found: .nothing)
        guard case .missing(_, let advice) = target else {
            Issue.record("expected a missing editor, got \(target)")
            return
        }
        #expect(advice.contains(typo))
        #expect(
            !advice.contains("terminal"),
            "a misspelled editor is not a terminal one: \(advice)"
        )
    }

    @Test func anApplicationEveryMacHasIsFound() {
        // The other half, and the reason the case above means anything: a
        // lookup that never finds anything would make every editor look like
        // a terminal program. TextEdit ships with macOS, by name and by
        // bundle identifier both.
        #expect(ComposeHandoff.isApplication("TextEdit"))
        #expect(ComposeHandoff.isApplication("com.apple.TextEdit"))
        #expect(ComposeHandoff.isApplication("  TextEdit  "), "what somebody typed is trimmed")
    }
}
