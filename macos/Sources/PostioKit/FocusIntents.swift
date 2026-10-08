import Foundation
import Observation
import PostioFFI

/// What Focus's controller says about the list, held for drawing
/// (specs/009-focus-macos T049; the plan called it `IntentApplier`).
///
/// The cursor, the selection, `!`'s heading and the toast are the
/// controller's (`crates/postio-focus`), the same one GTK's window drives:
/// keys, menus and clicks go to it through `invoke`, `focusPoint` and
/// `focusPick`, and its answers come back on `nextEvent` as five
/// `UiEvent`s. This is the one switch over them. It decides nothing -- a
/// field changed here that the event did not name would be the Mac
/// keeping an opinion of its own about where the keyboard is, which is the
/// bug the controller exists to end.
///
/// No AppKit: the table (`FocusListTable`) and the SwiftUI views
/// (`ActionBar`) both read it, and iOS must stay reachable (#1264).
@MainActor
@Observable
public final class FocusIntents {
    /// What one applied event changed, so the table redraws only that.
    public enum Change: Equatable, Sendable {
        /// The cursor moved off `previous`; `toTop` scrolls the list to its
        /// very top rather than just bringing the row into view.
        case cursor(previous: Int?, toTop: Bool)
        /// The selection or its words changed.
        case selection
        /// `!`'s single heading came or went.
        case heading
        /// Scroll the list to its top.
        case listToTop
        /// Something to say.
        case toast
    }

    /// Something said back: the host's words, how to draw them, and for
    /// how long.
    public struct Toast: Equatable, Sendable {
        public let text: String
        public let kind: ToastKindFfi
        public let undoable: Bool
        public let seconds: Double

        public init(text: String, kind: ToastKindFfi, undoable: Bool, seconds: Double) {
            self.text = text
            self.kind = kind
            self.undoable = undoable
            self.seconds = seconds
        }

        /// Only a completion the stack can take back offers Undo: an undo
        /// and a refusal changed nothing to return to.
        public var offersUndo: Bool { kind == .completed && undoable }

        /// How long a toast stays when the controller does not say: the
        /// numbers `Notice` has used since #1146, so the two never disagree
        /// while both are drawn. A completion with Undo outlasts the reach
        /// for the mouse; a refusal is a hint and goes soonest.
        public static func defaultSeconds(kind: ToastKindFfi, undoable: Bool) -> Double {
            switch kind {
            case .completed: return undoable ? 6 : 4
            case .undone: return 4
            case .notice: return 2
            }
        }
    }

    /// Everything held, as one value: what a test compares before and
    /// after an event to prove nothing else moved.
    public struct Snapshot: Equatable, Sendable {
        public var cursor: Int?
        public var selected: Set<Int64>
        public var everything: Bool
        public var summary: String?
        public var heading: String?
        public var toast: Toast?
    }

    /// The row the keyboard is on.
    public private(set) var cursor: Int?
    /// The messages selected, when the selection names them.
    public private(set) var selected: Set<Int64> = []
    /// Whether everything the view shows is selected (C19): a predicate,
    /// so rows whose pages have not arrived are selected too.
    public private(set) var everything = false
    /// "3 selected": the action bar's count, composed by the controller.
    public private(set) var summary: String?
    /// "Has action · 7" while `!` narrows the list; the day headings
    /// otherwise.
    public private(set) var heading: String?
    /// What is being said, until it is dismissed.
    public private(set) var toast: Toast?
    /// Bumped with every toast, so the same words twice are two toasts and
    /// an old timer cannot take down a newer one.
    public private(set) var toastToken = 0

    public init() {}

    public var snapshot: Snapshot {
        Snapshot(
            cursor: cursor, selected: selected, everything: everything, summary: summary,
            heading: heading, toast: toast)
    }

    /// Whether anything is selected: what shows the action bar.
    public var hasSelection: Bool { everything || !selected.isEmpty }

    /// Whether the row standing for `message` draws a checked box.
    public func isPicked(_ message: Int64) -> Bool {
        everything || selected.contains(message)
    }

    /// Apply `event` if it is one of the controller's, and say what it
    /// changed; `nil` for every other event, which changes nothing here.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusCursor(position, toTop):
            let previous = cursor
            cursor = Int(position)
            return .cursor(previous: previous, toTop: toTop)
        case let .focusSelection(ids, all, words):
            selected = Set(ids)
            everything = all
            summary = words
            return .selection
        case let .focusHeading(text):
            heading = text
            return .heading
        case .focusListToTop:
            return .listToTop
        case let .focusToast(text, kind, undoable, seconds):
            toast = Toast(
                text: text, kind: kind, undoable: undoable,
                seconds: seconds.map(Double.init)
                    ?? Toast.defaultSeconds(kind: kind, undoable: undoable))
            toastToken += 1
            return .toast
        default:
            return nil
        }
    }

    /// Take the toast down, if `token` is still the one showing.
    public func dismissToast(token: Int) {
        guard token == toastToken else { return }
        toast = nil
    }

    /// The list is `count` rows long now (`FocusListChanged`).
    ///
    /// Not a move of the controller's: until its first `FocusCursor` lands
    /// the ring stands on row 0, where the controller opens every list too
    /// (C30), and a cursor past the end of a shorter list is drawn on its
    /// last row until the controller says where it went.
    public func listResized(count: Int) {
        cursor = count == 0 ? nil : min(cursor ?? 0, count - 1)
    }
}
