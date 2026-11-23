import Foundation
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

@MainActor
@Suite struct ComposeHandoffTests {
    @Test func aTerminalEditorIsNotOpenedAndSaysWhy() {
        #expect(ComposeHandoff.found("vi") == .terminalProgram)

        let target = settingsHandoffTarget(configured: "vi", found: .terminalProgram)
        guard case .needsTerminal(_, let advice) = target else {
            Issue.record("expected a terminal program, got \(target)")
            return
        }
        #expect(advice.contains("terminal"))
    }

    @Test func anEditorThatIsNotThereSaysThatRatherThanBlamingTheTerminal() {
        // A name that is not an application is usually a terminal program, otherwise a typo
        // (#1297).
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
        #expect(ComposeHandoff.isApplication("TextEdit"))
        #expect(ComposeHandoff.isApplication("com.apple.TextEdit"))
        #expect(ComposeHandoff.isApplication("  TextEdit  "), "what somebody typed is trimmed")
    }
}
