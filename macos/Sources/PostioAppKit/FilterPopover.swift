import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The filter popovers' presenter (specs/010-focus-search T083; design
/// §3.6, screens 08 and 09): an `NSPopover` with its arrow, hung from the
/// filter button the controller opened it for, holding
/// `FilterPopoverContent`.
///
/// It opens and closes when `FilterPopoverModel` says the controller did,
/// and reports only the closes it made itself -- Esc, a click away -- once,
/// through the model; a close the controller asked for is never echoed.
@MainActor
public final class FilterPopover: NSObject, NSPopoverDelegate {
    private let model: FilterPopoverModel
    private var anchors: [FilterKindFfi: WeakAnchor] = [:]
    private var popover: NSPopover?
    /// The popover is being closed because the controller said so.
    private var closingQuietly = false

    public init(model: FilterPopoverModel) {
        self.model = model
        super.init()
        model.dismiss = { [weak self] in self?.closeQuietly() }
    }

    /// Whether a popover is up: what a demo's keys type into.
    public var isShown: Bool { popover?.isShown == true }

    /// The view a filter button's popover hangs from.
    func register(_ view: NSView, for kind: FilterKindFfi) {
        anchors[kind] = WeakAnchor(view: view)
    }

    /// Show what `change` says.
    public func apply(_ change: FilterPopoverModel.Change) {
        switch change {
        case let .open(kind): show(kind)
        case .redraw: break
        case .close: closeQuietly()
        }
    }

    private func show(_ kind: FilterKindFfi) {
        closeQuietly()
        guard let anchor = anchors[kind]?.view, anchor.window != nil else { return }
        let hosting = NSHostingController(rootView: FilterPopoverContent(model: model))
        hosting.sizingOptions = .preferredContentSize
        let popover = NSPopover()
        popover.contentViewController = hosting
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        self.popover = popover
        popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
    }

    private func closeQuietly() {
        guard let popover else { return }
        closingQuietly = true
        popover.close()
        closingQuietly = false
        self.popover = nil
    }

    public func popoverDidClose(_ notification: Notification) {
        guard !closingQuietly else { return }
        popover = nil
        // Esc or a click away: the toolkit closed it.
        model.closedByToolkit()
    }

    private struct WeakAnchor {
        weak var view: NSView?
    }
}

/// The view a filter button's popover hangs from: laid behind the button
/// (`FilterBarView`'s `anchor`), the size of it.
public struct FilterPopoverAnchor: NSViewRepresentable {
    let kind: FilterKindFfi
    let presenter: FilterPopover

    public init(kind: FilterKindFfi, presenter: FilterPopover) {
        self.kind = kind
        self.presenter = presenter
    }

    public func makeNSView(context: Context) -> NSView {
        let view = NSView()
        presenter.register(view, for: kind)
        return view
    }

    public func updateNSView(_ view: NSView, context: Context) {
        presenter.register(view, for: kind)
    }
}
