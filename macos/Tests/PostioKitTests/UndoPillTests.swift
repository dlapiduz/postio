import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The undo pill's words (specs/009-focus-macos T093, screen 15): what a
/// verb did, and Undo with its key while the stack can take it back.
@MainActor
struct UndoPillTests {
    static func toast(
        _ text: String = "Archived 3 messages", kind: ToastKindFfi = .completed,
        undoable: Bool = true, seconds: UInt32? = 8
    ) -> FocusIntents.Toast {
        let focus = FocusIntents()
        focus.apply(.focusToast(text: text, kind: kind, undoable: undoable, seconds: seconds))
        return focus.toast!
    }

    @Test func anUndoableCompletionOffersUndoWithItsKey() {
        let words = UndoPillWords(toast: Self.toast(), undoCap: "⌘Z")
        #expect(words.text == "Archived 3 messages")
        #expect(words.undo == "Undo")
        #expect(words.cap == "⌘Z")
        #expect(words.accessibility == "Archived 3 messages. Undo, ⌘Z")
    }

    @Test func anUndoOrARefusalOffersNothingToTakeBack() {
        for kind in [ToastKindFfi.undone, .notice] {
            let words = UndoPillWords(toast: Self.toast("Undone", kind: kind), undoCap: "⌘Z")
            #expect(words.undo == nil)
            #expect(words.cap == nil)
            #expect(words.accessibility == "Undone")
        }
    }

    @Test func aRebindingIsTheCapShown() {
        let words = UndoPillWords(toast: Self.toast(), undoCap: nil)
        #expect(words.undo == "Undo")
        #expect(words.cap == nil)
        #expect(words.accessibility == "Archived 3 messages. Undo")
    }

    @Test func aQueuedSendIsUndoneLikeAnyVerb() {
        // Its Undo is the ordinary `undo`: the controller cancels the send
        // first (T089), so the pill needs nothing of its own.
        let words = UndoPillWords(toast: Self.toast("Message queued to send"), undoCap: "⌘Z")
        #expect(words.undo == "Undo")
    }

    @Test func aToastStaysForTheControllersSeconds() {
        #expect(Self.toast(seconds: 30).seconds == 30)
        // Always said now; were it not, the controller's usual eight, never
        // a number the Mac made up per kind.
        #expect(Self.toast(kind: .undone, undoable: false, seconds: nil).seconds == 8)
        #expect(Self.toast(kind: .notice, undoable: false, seconds: nil).seconds == 8)
    }
}
