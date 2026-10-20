import Observation
import PostioFFI
import SwiftUI

/// What the known-sender popover tells the engine (specs/011-focus-contacts
/// S6). `PostioSession` is the one real answer; the protocol is what lets a
/// test drive `ContactPopoverModel` without a store.
public protocol ContactPopoverEngine: AnyObject {
    /// A click on a segment of the arrival control.
    func focusContactArrival(_ arrival: ContactArrivalFfi)
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
}

extension PostioSession: ContactPopoverEngine {}

/// The known-sender popover, as the controller's last `FocusContactPopover`
/// drew it (handoff screen 04): the avatar, name, address and role, three
/// tiles, up to two open lines, the arrival control and the three actions.
///
/// Every word, the avatar's letters and colour, the tiles and their
/// captions are Rust's (`postio_ui::contact_popover`); this adapts the
/// records and sends two things back, a click on a segment and a click on
/// an action. The keys are commands the controller resolves.
///
/// No AppKit (#1264): `ContactPopover` presents it and
/// `ContactPopoverView` draws it.
@MainActor
@Observable
public final class ContactPopoverModel {
    /// What one applied event changed.
    public enum Change: Equatable {
        /// It opened: hang it from the sender's name.
        case open
        /// It was drawn again, in place.
        case redraw
        /// The controller put it away.
        case close
    }

    /// What it draws; `nil` while it is closed.
    public private(set) var view: ContactPopoverViewFfi?

    @ObservationIgnored private let engine: ContactPopoverEngine

    public init(engine: ContactPopoverEngine) {
        self.engine = engine
    }

    /// Whether it is up.
    public var isOpen: Bool { view != nil }

    /// Apply `event` if it is the popover's, and say what it changed; `nil`
    /// for every other event.
    @discardableResult
    public func apply(_ event: UiEvent) -> Change? {
        switch event {
        case let .focusContactPopover(next):
            let change: Change = view == nil ? .open : .redraw
            view = next
            return change
        case .focusCloseSurface(.contactPopover):
            guard view != nil else { return nil }
            view = nil
            return .close
        default:
            return nil
        }
    }

    /// A click on a segment of the arrival control.
    public func choose(_ arrival: ContactArrivalFfi) {
        guard view != nil else { return }
        engine.focusContactArrival(arrival)
    }

    /// A click on a button along the foot: the command a key would send.
    public func run(_ action: PopoverActionFfi) {
        guard view != nil, action.enabled else { return }
        engine.invoke(action.command)
    }

    /// The toolkit took it down: a click away. The controller is told by
    /// whoever presents it.
    public func closedByToolkit() {
        view = nil
    }

    /// The avatar's fill.
    public static func colour(_ rgb: UInt32) -> Color {
        Color(
            red: Double((rgb >> 16) & 0xff) / 255,
            green: Double((rgb >> 8) & 0xff) / 255,
            blue: Double(rgb & 0xff) / 255)
    }
}
