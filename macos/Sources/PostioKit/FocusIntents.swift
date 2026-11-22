import Foundation
import Observation
import PostioFFI

/// The controller owns the cursor, selection, `!` heading and toast; this decides nothing: a field
/// changed here that the
/// event did not name would be the Mac keeping its own opinion of where the keyboard is. No AppKit
/// (#1264).
@MainActor
@Observable
public final class FocusIntents {
    public enum Change: Equatable, Sendable {
        case cursor(previous: Int?, toTop: Bool)
        case selection
        case heading
        case listToTop
        case toast
    }

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

        public var offersUndo: Bool { kind == .completed && undoable }

        /// Every `FocusToast` says its duration, so this is reached only by an engine older than
        /// the slice.
        public static let usualSeconds: Double = 8
    }

    public struct Snapshot: Equatable, Sendable {
        public var cursor: Int?
        public var selected: Set<Int64>
        public var everything: Bool
        public var summary: String?
        public var heading: String?
        public var toast: Toast?
    }

    public private(set) var cursor: Int?
    public private(set) var selected: Set<Int64> = []
    /// A predicate, so rows whose pages have not arrived are selected too (C19).
    public private(set) var everything = false
    public private(set) var summary: String?
    public private(set) var heading: String?
    public private(set) var toast: Toast?
    public private(set) var toastToken = 0

    public init() {}

    public var snapshot: Snapshot {
        Snapshot(
            cursor: cursor, selected: selected, everything: everything, summary: summary,
            heading: heading, toast: toast)
    }

    public var hasSelection: Bool { everything || !selected.isEmpty }

    public func isPicked(_ message: Int64) -> Bool {
        everything || selected.contains(message)
    }

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
                seconds: seconds.map(Double.init) ?? Toast.usualSeconds)
            toastToken += 1
            return .toast
        default:
            return nil
        }
    }

    public enum Surface: Equatable {
        case openMessage(message: Int64, index: UInt32, total: UInt32)
        case openDigest(delivery: Int64)
        case close(SurfaceKindFfi)
        case reader(ReaderVerbFfi)
        case keyboardHome
        case thread(ThreadSaid)
    }

    public enum ThreadSaid: Equatable {
        case bar(ThreadBarFfi)
        case kicker(String)
        case olderBanner(ThreadBannerFfi?)
        case newReply(ThreadBannerFfi?)
        case replyTo(String?)
        case matchBanner(MatchBannerFfi?)
    }

    public static func surface(_ event: UiEvent) -> Surface? {
        switch event {
        case let .focusOpenMessage(message, index, total):
            return .openMessage(message: message, index: index, total: total)
        case let .focusOpenDigest(delivery):
            return .openDigest(delivery: delivery)
        case let .focusCloseSurface(kind):
            return .close(kind)
        case let .focusReader(verb):
            return .reader(verb)
        case .focusKeyboardHome:
            return .keyboardHome
        case let .focusThreadBar(view):
            return .thread(.bar(view))
        case let .focusKicker(text):
            return .thread(.kicker(text))
        case let .focusOlderBanner(banner):
            return .thread(.olderBanner(banner))
        case let .focusNewReply(banner):
            return .thread(.newReply(banner))
        case let .focusReplyTo(text):
            return .thread(.replyTo(text))
        case let .focusMatchBanner(banner):
            return .thread(.matchBanner(banner))
        default:
            return nil
        }
    }

    public func dismissToast(token: Int) {
        guard token == toastToken else { return }
        toast = nil
    }

    /// Not a move of the controller's: until its first `FocusCursor` lands the ring stands on row 0
    /// (C30), and a cursor past the
    /// end of a shorter list is drawn on its last row until the controller says where it went.
    public func listResized(count: Int) {
        cursor = count == 0 ? nil : min(cursor ?? 0, count - 1)
    }
}
