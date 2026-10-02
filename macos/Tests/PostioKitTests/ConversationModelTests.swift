import PostioFFI
import Testing

@testable import PostioKit

/// What the pane holds while somebody reads a conversation (#1263).
///
/// The fold a conversation *opens* with is the boundary's and is tested
/// there. What is tested here is the part that is genuinely this frontend's:
/// what happens as someone moves, folds and expands -- which, with the pane one
/// document, is what the page is asked to do -- and that none of it quietly
/// re-decides the fold.
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
            // A received message is in no send state. `draft: Bool` became
            // `sendState: String?` when a row learned to say *which* state a
            // message it is sending is in.
            sendState: nil,
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
        #expect(model.focus == nil)
    }


    // -- landing on a message with no thread (#70's shape again) -----------

    @Test func aPaneWithNothingToShowShowsNothing() {
        // `messages.thread_id` is nullable — `ON DELETE SET NULL`, and
        // `threads.rs` sets it to NULL outright — so a row whose `thread` is
        // `nil` is reachable, and landing on one has to *empty* the pane.
        //
        // There was no way to: `show(_:)` was the only writer, so the last
        // conversation stayed drawn under the new selection. The boundary
        // guards the other half of exactly this and says why — *"a pane still
        // drawing the previous conversation under a new selection is worse
        // than an empty one, because it looks like an answer"*. This side did
        // not.
        let model = ConversationModel()
        model.show(conversation())
        #expect(model.conversation != nil)

        model.clear()

        #expect(model.conversation == nil, "the pane is showing somebody else's mail")
        #expect(model.rows.isEmpty)
        #expect(model.subject.isEmpty)
        #expect(model.meta.isEmpty)
    }

    @Test func clearingForgetsWhatWasDoneToTheLastConversation() {
        // Everything `show` resets, `clear` has to reset too, or the next
        // conversation opens wearing the previous one's expansions.
        let model = ConversationModel()
        model.show(conversation())
        model.expandAll()
        model.focusNext()

        model.clear()
        model.show(conversation())

        #expect(model.expanded == [false, false, false, false, true, true])
        #expect(model.focused == 4, "the fold and the focus are the boundary's, freshly")
    }

    @Test func clearingAnAlreadyEmptyPaneIsNotAnError() {
        let model = ConversationModel()
        model.clear()
        #expect(model.conversation == nil)
    }

    // -- the document does what the keys ask (#1595) ----------------------

    @Test func jAndKScrollTheDocumentToTheMessageTheyLandOn() {
        // In one document, moving inside a conversation is scrolling it: the
        // focus changing with nothing on screen moving is a key that looks
        // broken.
        let model = ConversationModel()
        model.show(conversation())
        model.focusPrevious()
        #expect(model.documentRequest?.message == 4)
        #expect(model.documentRequest?.isScroll == true)
        model.focusNext()
        #expect(model.documentRequest?.message == 5)
    }

    @Test func foldingActsOnTheFocusedMessageInTheDocument() {
        let model = ConversationModel()
        model.show(conversation())
        model.toggleFocused()
        #expect(model.documentRequest?.action == .toggle(message: 5))
    }

    @Test func expandAllOpensEveryMessageInTheDocument() {
        let model = ConversationModel()
        model.show(conversation())
        model.expandAll()
        #expect(model.documentRequest?.action == .expandAll)
    }

    @Test func theSameRequestTwiceIsTwoRequests() {
        // `z` twice folds and unfolds: a request keyed only on what it asks
        // would look unchanged the second time and be swallowed.
        let model = ConversationModel()
        model.show(conversation())
        model.toggleFocused()
        let first = model.documentRequest?.serial
        model.toggleFocused()
        #expect(model.documentRequest?.serial != first)
    }

    @Test func aNewConversationForgetsTheLastOnesRequest() {
        let model = ConversationModel()
        model.show(conversation())
        model.toggleFocused()
        model.show(conversation())
        #expect(model.documentRequest == nil)
    }

    // -- the rail (#1576, #1595) --------------------------------------------

    @Test func theRailStartsMarkedWhereThePaneOpens() {
        // The pane lands on the boundary's focus; the rail says so without
        // asking the page to go anywhere it is not already going.
        let model = ConversationModel()
        model.show(conversation())
        #expect(model.marked == 4)
        #expect(model.documentRequest == nil)
    }

    @Test func choosingARowMarksItAndTakesThePaneThere() {
        let model = ConversationModel()
        model.show(conversation())
        model.choose(1)
        #expect(model.marked == 1)
        #expect(model.focus == 4, "the boundary's opening focus is not what moved")
        #expect(model.documentRequest?.message == 2)
        #expect(model.documentRequest?.settle != nil, "the page must say when it got there")
    }

    @Test func theObserverMovesTheMarkAndNotThePane() {
        let model = ConversationModel()
        model.show(conversation())
        model.observed(message: 2)
        #expect(model.marked == 1)
        #expect(model.documentRequest == nil, "the pane is already there")
    }

    @Test func theObserverWaitsWhileThePaneIsBeingTakenSomewhere() {
        // Mid-scroll it sees the messages the pane passes over; listened to,
        // the mark would land short of the one the reader chose.
        let model = ConversationModel()
        model.show(conversation())
        model.choose(0)
        let settle = try? #require(model.documentRequest?.settle)
        model.observed(message: 3)
        #expect(model.marked == 0)
        if let settle { model.settled(settle) }
        model.observed(message: 3)
        #expect(model.marked == 2)
    }

    @Test func foldingActsOnTheMarkedMessage() {
        // `z` means the message the reader is on -- which the observer keeps
        // current as they scroll, not only the keys.
        let model = ConversationModel()
        model.show(conversation())
        model.observed(message: 2)
        model.toggleFocused()
        #expect(model.documentRequest?.action == .toggle(message: 2))
    }
}

