import PostioFFI
import Testing

@testable import PostioKit

/// What Postio says back when you ask it to do something.
///
/// Four core events are the *outcome* of an action — it ran, it was taken
/// back, it was refused, it failed — and every one of them was dropped at the
/// boundary. So on macOS: a send that failed said nothing, `a` with nothing
/// selected said nothing, and an archive that could be undone never offered
/// to be. The application did the work and never answered.
///
/// This is the decision half — how long a notice stays, whether it offers
/// Undo, and which of two notices wins when both are in flight. `Engine` is
/// untestable by construction, so anything decided there is decided without
/// a test; this is what keeps that from happening again.
@Suite struct NoticeTests {
    @Test func aCompletedActionThatCanBeTakenBackOffersIt() {
        let notice = Notice(kind: .completed, message: "Archived 12 messages", undoable: true)
        #expect(notice.offersUndo)
        #expect(notice.message == "Archived 12 messages")
    }

    @Test func nothingButACompletionEverOffersUndo() {
        // A refusal changed nothing and a failure did not finish, so there is
        // nothing to return to. The boundary already says `undoable: false`
        // for those; this is the belt to that brace, because an Undo button
        // that undoes nothing is worse than none.
        for kind in [NoticeKindFfi.undone, .refused, .failed] {
            let notice = Notice(kind: kind, message: "…", undoable: true)
            #expect(!notice.offersUndo, "\(kind) offered an undo")
        }
    }

    @Test func aRefusalIsQuieterAndShorterThanAFailure() {
        // A refusal is the user asking for something that does not apply,
        // which is an ordinary thing to do with a keyboard: it gets a hint.
        // A failure is something that went wrong without being asked for, and
        // has to be hard to miss.
        let refused = Notice(kind: .refused, message: "Nothing is selected", undoable: false)
        let failed = Notice(kind: .failed, message: "The server refused the password", undoable: false)
        #expect(refused.seconds < failed.seconds)
        #expect(refused.isAlarming == false)
        #expect(failed.isAlarming == true)
    }

    @Test func anUndoableCompletionStaysLongEnoughToPress() {
        // PRODUCT.md's undo window. A toast that offers Undo and goes before
        // anybody can reach it is a promise the application does not keep.
        let notice = Notice(kind: .completed, message: "Archived", undoable: true)
        #expect(notice.seconds >= 5)
    }

    @Test func aFailureOutranksEverythingElseOnScreen() {
        // Two notices in flight is ordinary — an action completes while a
        // sync fails. The one that has to be read wins.
        let failed = Notice(kind: .failed, message: "Send failed", undoable: false)
        let completed = Notice(kind: .completed, message: "Archived", undoable: true)
        #expect(Notice.winner(showing: completed, arriving: failed) == failed)
        #expect(Notice.winner(showing: failed, arriving: completed) == failed)
    }

    @Test func anUndoReplacesTheCompletionItTookBack() {
        // *Archived 12 messages* → press `u` → *Archived 12 messages, undone*.
        // The second is the answer to the first and must replace it rather
        // than queue behind it.
        let completed = Notice(kind: .completed, message: "Archived 12", undoable: true)
        let undone = Notice(kind: .undone, message: "Archived 12, undone", undoable: false)
        #expect(Notice.winner(showing: completed, arriving: undone) == undone)
    }

    @Test func anEventBecomesANotice() {
        let event = UiEvent.notice(
            kind: .completed, message: "Archived 12 messages", undoable: true
        )
        #expect(Notice(event) == Notice(kind: .completed, message: "Archived 12 messages", undoable: true))
        #expect(Notice(.pageReady(page: 0)) == nil, "only a notice is a notice")
    }

    @Test func theUndoButtonRunsTheRegistrysUndo() {
        // The button and `u` have to be one command. A literal that stops
        // naming a registered command is a button that silently does
        // nothing — the same failure `everyInterceptedIdNamesARealCommand`
        // guards on the other list.
        let known = Set(PostioRegistry.commands.map(\.id))
        #expect(known.contains(Notice.undoCommand))
    }
}
