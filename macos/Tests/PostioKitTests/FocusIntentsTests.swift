import PostioFFI
import Testing

@testable import PostioKit

/// What the controller says about the list, applied to the model the table
/// and the bar read (specs/009-focus-macos T048, T049).
///
/// Each intent changes exactly what it says and nothing else: a cursor that
/// moved must not clear the selection, a selection must not move the
/// cursor, and a toast must leave the list alone. The controller decides;
/// the Mac only holds the answer, so a field changed here that the event
/// did not name is the Mac having an opinion of its own.
@MainActor
struct FocusIntentsTests {
    /// A model with something in every field, so "nothing else changed" is
    /// a claim about real values rather than about defaults.
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
        // C19: `X` selects every conversation in the view, including the
        // ones whose pages have not arrived; it names none of them.
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
            seconds: FocusIntents.Toast.defaultSeconds(kind: .undone, undoable: false))
        #expect(focus.snapshot == expected)
        #expect(change == .toast)
    }

    @Test func aToastKeepsTheSecondsItWasGiven() {
        // An answer's Undo lasts as long as its reply waits (FR-102).
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
        // Archived, then Archived again: two things happened.
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
        // C30, drawn before the first FocusCursor arrives; the controller
        // opens every list on row 0 as well, so the two never disagree.
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
}
