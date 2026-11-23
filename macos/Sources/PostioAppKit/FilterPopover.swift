import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// Reports only the closes it made itself (Esc, a click away), once, through the model; a close the
/// controller asked for is never echoed.
@MainActor
public final class FilterPopover: NSObject, NSPopoverDelegate {
    private let model: FilterPopoverModel
    private var anchors: [FilterKindFfi: WeakAnchor] = [:]
    private var popover: NSPopover?
    private var closingQuietly = false

    public init(model: FilterPopoverModel) {
        self.model = model
        super.init()
        model.dismiss = { [weak self] in self?.closeQuietly() }
    }

    public var isShown: Bool { popover?.isShown == true }

    func register(_ view: NSView, for kind: FilterKindFfi) {
        anchors[kind] = WeakAnchor(view: view)
    }

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
        model.closedByToolkit()
    }

    private struct WeakAnchor {
        weak var view: NSView?
    }
}

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
