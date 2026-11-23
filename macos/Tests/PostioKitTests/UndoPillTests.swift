import Foundation
import PostioFFI
import Testing

@testable import PostioKit

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
        let words = UndoPillWords(toast: Self.toast("Message queued to send"), undoCap: "⌘Z")
        #expect(words.undo == "Undo")
    }

    @Test func aToastStaysForTheControllersSeconds() {
        #expect(Self.toast(seconds: 30).seconds == 30)
        #expect(Self.toast(kind: .undone, undoable: false, seconds: nil).seconds == 8)
        #expect(Self.toast(kind: .notice, undoable: false, seconds: nil).seconds == 8)
    }
}
