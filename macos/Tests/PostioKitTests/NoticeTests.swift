import PostioFFI
import Testing

@testable import PostioKit

@Suite struct NoticeTests {
    @Test func aCompletedActionThatCanBeTakenBackOffersIt() {
        let notice = Notice(kind: .completed, message: "Archived 12 messages", undoable: true)
        #expect(notice.offersUndo)
        #expect(notice.message == "Archived 12 messages")
    }

    @Test func nothingButACompletionEverOffersUndo() {
        // The boundary already says `undoable: false`; this is the belt, since an Undo that undoes
        // nothing is worse than none.
        for kind in [NoticeKindFfi.undone, .refused, .failed] {
            let notice = Notice(kind: kind, message: "…", undoable: true)
            #expect(!notice.offersUndo, "\(kind) offered an undo")
        }
    }

    @Test func aRefusalIsQuieterAndShorterThanAFailure() {
        let refused = Notice(kind: .refused, message: "Nothing is selected", undoable: false)
        let failed = Notice(kind: .failed, message: "The server refused the password", undoable: false)
        #expect(refused.seconds < failed.seconds)
        #expect(refused.isAlarming == false)
        #expect(failed.isAlarming == true)
    }

    @Test func anUndoableCompletionStaysLongEnoughToPress() {
        // A toast offering Undo that goes before anyone can reach it is a promise not kept.
        let notice = Notice(kind: .completed, message: "Archived", undoable: true)
        #expect(notice.seconds >= 5)
    }

    @Test func aFailureOutranksEverythingElseOnScreen() {
        let failed = Notice(kind: .failed, message: "Send failed", undoable: false)
        let completed = Notice(kind: .completed, message: "Archived", undoable: true)
        #expect(Notice.winner(showing: completed, arriving: failed) == failed)
        #expect(Notice.winner(showing: failed, arriving: completed) == failed)
    }

    @Test func anUndoReplacesTheCompletionItTookBack() {
        // The undo notice answers the completion and replaces it rather than queueing.
        let completed = Notice(kind: .completed, message: "Archived 12", undoable: true)
        let undone = Notice(kind: .undone, message: "Archived 12, undone", undoable: false)
        #expect(Notice.winner(showing: completed, arriving: undone) == undone)
    }

    @Test func anEventBecomesANotice() {
        let event = UiEvent.notice(
            kind: .completed, message: "Archived 12 messages", undoable: true
        )
        #expect(Notice(event) == Notice(kind: .completed, message: "Archived 12 messages", undoable: true))
        #expect(Notice(.keymapChanged) == nil, "only a notice is a notice")
    }

    @Test func theUndoButtonRunsTheRegistrysUndo() {
        // A literal that stops naming a registered command is a button that does nothing.
        let known = Set(PostioRegistry.commands.map(\.id))
        #expect(known.contains(Notice.undoCommand))
    }

    @Test func inFocusOnlyAFailureIsANoticeTheRestAreTheControllersToast() {
        // The controller says these as `FocusToast`; drawn as notices too, one archive would be
        // announced twice.
        for kind in [NoticeKindFfi.completed, .undone, .refused] {
            #expect(!Notice(kind: kind, message: "", undoable: false).shownBesideFocusToast)
        }
        #expect(Notice(kind: .failed, message: "", undoable: false).shownBesideFocusToast)
    }
}
