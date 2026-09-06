import PostioFFI
import Testing

@testable import PostioKit

/// What the pane holds while somebody reads a conversation (#1263).
///
/// The fold a conversation *opens* with is the boundary's and is tested
/// there. What is tested here is the part that is genuinely this frontend's:
/// what happens as someone expands, collapses and reveals — and that none of
/// it quietly re-decides the fold.
@MainActor
@Suite struct ConversationModelTests {
    private func row(_ id: Int64, from sender: String = "Ada") -> RowFfi {
        RowFfi(
            id: id,
            thread: 7,
            isThread: false,
            from: sender,
            fromAddress: "\(sender.lowercased())@example.com",
            initials: String(sender.prefix(1)),
            subject: "Radon reduction",
            preview: "Snippet",
            receivedAt: 1_770_000_000 + id,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            hasAttachments: false,
            threadCount: 6,
            participants: ""
        )
    }

    /// Six messages, the last two open — the shape the canvas draws.
    private func conversation(
        expanded: [Bool] = [false, false, false, false, true, true]
    ) -> ConversationFfi {
        ConversationFfi(
            thread: 7,
            subject: "Radon reduction",
            meta: "6 messages · Ada, Bo · 10-25 Aug",
            rows: (1...6).map { row(Int64($0), from: $0 % 2 == 0 ? "Bo" : "Ada") },
            focus: 4,
            expanded: expanded
        )
    }

    @Test func theModelOpensWithTheFoldTheBoundaryDecided() {
        let model = ConversationModel()
        model.show(conversation())

        #expect(model.expanded == [false, false, false, false, true, true])
        #expect(model.focus == 4)
    }

    @Test func expandingAMessageLeavesTheRestAlone() {
        let model = ConversationModel()
        model.show(conversation())

        model.toggle(0)

        #expect(model.expanded == [true, false, false, false, true, true])
    }

    @Test func collapsingIsTheSameGesture() {
        let model = ConversationModel()
        model.show(conversation())

        model.toggle(5)

        #expect(model.expanded == [false, false, false, false, true, false])
    }

    @Test func expandAllOpensEveryMessage() {
        let model = ConversationModel()
        model.show(conversation())

        model.expandAll()

        #expect(model.expanded.allSatisfy { $0 })
        #expect(model.runs.isEmpty, "nothing is folded, so nothing is a divider")
    }

    @Test func aRunOfCollapsedMessagesIsHiddenBehindItsDivider() {
        let model = ConversationModel()
        model.show(conversation())

        // Four collapsed in a row: one divider, and none of them drawn.
        #expect(model.runs.count == 1)
        #expect(model.runs[0].start == 0)
        #expect(model.runs[0].count == 4)
        #expect(model.isVisible(0) == false)
        #expect(model.isVisible(3) == false)
        #expect(model.isVisible(4), "an expanded message is never inside a run")
    }

    @Test func showingARunRevealsItsMessagesWithoutExpandingThem() {
        // "Show" is about the divider, not about the bodies: five one-line
        // headers appear, and none of them costs a web view.
        let model = ConversationModel()
        model.show(conversation())
        let run = model.runs[0]

        model.reveal(run)

        #expect(model.isVisible(0))
        #expect(model.isVisible(3))
        #expect(model.expanded == [false, false, false, false, true, true])
        #expect(model.runs.isEmpty, "a revealed run no longer draws a divider")
    }

    @Test func aShortRunIsNeverFolded() {
        // Two collapsed messages are two lines. A divider that saves nothing
        // is a gesture where there was none.
        let model = ConversationModel()
        model.show(conversation(expanded: [true, false, false, true, true, true]))

        #expect(model.runs.isEmpty)
        #expect(model.isVisible(1))
    }

    @Test func openingAnotherConversationForgetsTheLastOne() {
        // A pane still drawing the previous conversation's revealed runs
        // under a new selection looks like an answer.
        let model = ConversationModel()
        model.show(conversation())
        model.reveal(model.runs[0])
        model.toggle(0)

        model.show(conversation())

        #expect(model.expanded == [false, false, false, false, true, true])
        #expect(model.isVisible(0) == false)
    }

    // -- the keyboard inside a conversation (J / K / Space) ----------------

    @Test func theFocusStartsWhereTheBoundaryPutIt() {
        let model = ConversationModel()
        model.show(conversation())

        #expect(model.focused == 4, "the pane opens on the first unread")
    }

    @Test func walkingTheConversationMovesTheFocusAndStops() {
        let model = ConversationModel()
        model.show(conversation())

        model.focusNext()
        #expect(model.focused == 5)
        model.focusNext()
        #expect(model.focused == 5, "the end of a conversation is the end")

        model.focusPrevious()
        #expect(model.focused == 4)
    }

    @Test func foldingActsOnTheFocusedMessage() {
        // Space folds *the one you are looking at*, which is the only message
        // the keyboard can mean.
        let model = ConversationModel()
        model.show(conversation())

        model.toggleFocused()

        #expect(model.expanded == [false, false, false, false, false, true])
    }

    @Test func aConversationWithNothingInItHasNothingToFocus() {
        let model = ConversationModel()
        model.show(
            ConversationFfi(
                thread: 9, subject: "", meta: "", rows: [], focus: nil, expanded: []
            )
        )

        model.focusNext()
        model.toggleFocused()

        #expect(model.focused == 0)
        #expect(model.expanded.isEmpty)
    }

    @Test func aConversationWithNothingInItDrawsNothing() {
        let model = ConversationModel()
        model.show(
            ConversationFfi(
                thread: 9, subject: "", meta: "", rows: [], focus: nil, expanded: []
            )
        )

        #expect(model.rows.isEmpty)
        #expect(model.runs.isEmpty)
        #expect(model.focus == nil)
    }
}
