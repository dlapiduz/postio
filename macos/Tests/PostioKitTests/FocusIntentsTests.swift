import PostioFFI
import Testing

@testable import PostioKit

@MainActor
struct FocusIntentsTests {
    static func busy() -> FocusIntents {
        let focus = FocusIntents()
        focus.listResized(count: 20)
        focus.apply(.focusCursor(position: 4, toTop: false))
        focus.apply(.focusSelection(selected: [11, 12], everything: false, summary: "2 selected"))
        focus.apply(.focusHeading(text: "Has action \u{b7} 7"))
        focus.apply(.focusToast(text: "Archived", kind: .completed, undoable: true, seconds: nil))
        return focus
    }

    @Test func aCursorIntentMovesTheCursorAndNothingElse() {
        let focus = Self.busy()
        var expected = focus.snapshot
        let change = focus.apply(.focusCursor(position: 9, toTop: false))
        expected.cursor = 9
        #expect(focus.snapshot == expected)
        #expect(change == .cursor(previous: 4, toTop: false))
    }

    @Test func aCursorIntentSaysWhetherTheListGoesToItsTop() {
        let focus = Self.busy()
        #expect(focus.apply(.focusCursor(position: 0, toTop: true)) == .cursor(previous: 4, toTop: true))
        #expect(focus.cursor == 0)
    }

    @Test func aSelectionIntentReplacesTheSelectionAndItsWordsOnly() {
        let focus = Self.busy()
        var expected = focus.snapshot
        let change = focus.apply(
            .focusSelection(selected: [11, 12, 13], everything: false, summary: "3 selected"))
        expected.selected = [11, 12, 13]
        expected.summary = "3 selected"
        #expect(focus.snapshot == expected)
        #expect(change == .selection)
        #expect(focus.cursor == 4, "x never moves the cursor (invariant 1)")
    }

    @Test func everythingIsAPredicateNotAListOfIds() {
        // `X` selects every conversation in the view, including those whose pages have not arrived;
        // it names none.
        let focus = Self.busy()
        focus.apply(.focusSelection(selected: [], everything: true, summary: "All 312 selected"))
        #expect(focus.everything)
        #expect(focus.selected.isEmpty)
        #expect(focus.hasSelection)
        #expect(focus.isPicked(999), "a row nobody has seen is picked too")
    }

    @Test func aClearedSelectionHasNoWords() {
        let focus = Self.busy()
        focus.apply(.focusSelection(selected: [], everything: false, summary: nil))
        #expect(!focus.hasSelection)
        #expect(focus.summary == nil)
        #expect(!focus.isPicked(11))
    }

    @Test func aHeadingIntentReplacesTheHeadingOnly() {
        let focus = Self.busy()
        var expected = focus.snapshot
        let change = focus.apply(.focusHeading(text: nil))
        expected.heading = nil
        #expect(focus.snapshot == expected)
        #expect(change == .heading)
    }

    @Test func goingToTheTopIsAnInstructionAndHoldsNothing() {
        let focus = Self.busy()
        let before = focus.snapshot
        #expect(focus.apply(.focusListToTop) == .listToTop)
        #expect(focus.snapshot == before)
    }

    @Test func aToastIntentSaysSomethingAndLeavesTheListAlone() {
        let focus = Self.busy()
        var expected = focus.snapshot
        let change = focus.apply(
            .focusToast(text: "Undone: Archived 3", kind: .undone, undoable: false, seconds: nil))
        expected.toast = FocusIntents.Toast(
            text: "Undone: Archived 3", kind: .undone, undoable: false,
            seconds: FocusIntents.Toast.usualSeconds)
        #expect(focus.snapshot == expected)
        #expect(change == .toast)
    }

    @Test func aToastKeepsTheSecondsItWasGiven() {
        let focus = FocusIntents()
        focus.apply(.focusToast(text: "Accepted", kind: .completed, undoable: true, seconds: 30))
        #expect(focus.toast?.seconds == 30)
        #expect(focus.toast?.offersUndo == true)
    }

    @Test func onlyAnUndoableCompletionOffersUndo() {
        #expect(FocusIntents.Toast(text: "", kind: .completed, undoable: false, seconds: 1).offersUndo == false)
        #expect(FocusIntents.Toast(text: "", kind: .undone, undoable: true, seconds: 1).offersUndo == false)
        #expect(FocusIntents.Toast(text: "", kind: .notice, undoable: true, seconds: 1).offersUndo == false)
    }

    @Test func theSameToastTwiceIsTwoToasts() {
        let focus = FocusIntents()
        focus.apply(.focusToast(text: "Archived", kind: .completed, undoable: true, seconds: nil))
        let first = focus.toastToken
        focus.apply(.focusToast(text: "Archived", kind: .completed, undoable: true, seconds: nil))
        #expect(focus.toastToken != first)
    }

    @Test func dismissingAnOldToastLeavesANewerOneUp() {
        let focus = FocusIntents()
        focus.apply(.focusToast(text: "Archived", kind: .completed, undoable: true, seconds: nil))
        let old = focus.toastToken
        focus.apply(.focusToast(text: "Snoozed", kind: .completed, undoable: true, seconds: nil))
        focus.dismissToast(token: old)
        #expect(focus.toast?.text == "Snoozed")
        focus.dismissToast(token: focus.toastToken)
        #expect(focus.toast == nil)
    }

    @Test func anEventThatIsNotFocussIsNotApplied() {
        let focus = Self.busy()
        let before = focus.snapshot
        #expect(focus.apply(.focusPageReady(page: 0)) == nil)
        #expect(focus.apply(.keymapChanged) == nil)
        #expect(focus.snapshot == before)
    }

    @Test func untilTheControllerSpeaksTheCursorStandsOnTheFirstRow() {
        let focus = FocusIntents()
        #expect(focus.cursor == nil)
        focus.listResized(count: 3)
        #expect(focus.cursor == 0)
        focus.listResized(count: 0)
        #expect(focus.cursor == nil)
    }

    @Test func aShorterListKeepsTheCursorInsideIt() {
        let focus = FocusIntents()
        focus.listResized(count: 10)
        focus.apply(.focusCursor(position: 8, toTop: false))
        focus.listResized(count: 5)
        #expect(focus.cursor == 4)
    }

    @Test func theOpenMessagesIntentsAreSurfacesNotTheLists() {
        // A window over the list must not move the cursor or the selection.
        let focus = Self.busy()
        let before = focus.snapshot
        let events: [UiEvent] = [
            .focusOpenMessage(message: 7, index: 2, total: 9),
            .focusOpenDigest(delivery: 3),
            .focusCloseSurface(kind: .message),
            .focusReader(verb: .switchTreatment),
            .focusKeyboardHome,
        ]
        for event in events {
            #expect(focus.apply(event) == nil)
        }
        #expect(focus.snapshot == before)
        let expected: [FocusIntents.Surface?] = [
            .openMessage(message: 7, index: 2, total: 9),
            .openDigest(delivery: 3),
            .close(.message),
            .reader(.switchTreatment),
            .keyboardHome,
        ]
        #expect(events.map(FocusIntents.surface) == expected)
    }

    @Test func aListEventIsNoSurfaces() {
        #expect(FocusIntents.surface(.focusCursor(position: 1, toTop: false)) == nil)
        #expect(FocusIntents.surface(.focusPageReady(page: 0)) == nil)
        #expect(FocusIntents.surface(.keymapChanged) == nil)
    }
}
