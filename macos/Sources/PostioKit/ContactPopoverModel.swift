import Observation
import PostioFFI
import SwiftUI

public protocol ContactPopoverEngine: AnyObject {
    func focusContactArrival(_ arrival: ContactArrivalFfi)
    func invoke(_ id: String)
}

extension PostioSession: ContactPopoverEngine {}

/// Every word is Rust's (`postio_ui::contact_popover`); this adapts the records and sends back a
/// click on a segment or an action. No AppKit (#1264).
@MainActor
@Observable
public final class ContactPopoverModel {
    public enum Change: Equatable {
        case open
        case redraw
        case close
    }

    public private(set) var view: ContactPopoverViewFfi?

    @ObservationIgnored private let engine: ContactPopoverEngine

    public init(engine: ContactPopoverEngine) {
        self.engine = engine
    }

    public var isOpen: Bool { view != nil }

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

    public func choose(_ arrival: ContactArrivalFfi) {
        guard view != nil else { return }
        engine.focusContactArrival(arrival)
    }

    public func run(_ action: PopoverActionFfi) {
        guard view != nil, action.enabled else { return }
        engine.invoke(action.command)
    }

    public func closedByToolkit() {
        view = nil
    }

    public static func colour(_ rgb: UInt32) -> Color {
        Color(
            red: Double((rgb >> 16) & 0xff) / 255,
            green: Double((rgb >> 8) & 0xff) / 255,
            blue: Double(rgb & 0xff) / 255)
    }
}
