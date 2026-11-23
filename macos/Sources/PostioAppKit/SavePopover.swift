import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// A close the toolkit made (a click away) writes nothing.
@MainActor
public final class SavePopover: NSObject, NSPopoverDelegate {
    private let model: SavePopoverModel
    private var popover: NSPopover?
    private var closingQuietly = false
    public weak var anchor: NSView?

    public init(model: SavePopoverModel) {
        self.model = model
        super.init()
        model.dismiss = { [weak self] in self?.closeQuietly() }
    }

    public var isShown: Bool { popover?.isShown == true }

    public func apply(_ change: SavePopoverModel.Change) {
        switch change {
        case .open: show()
        case .close: closeQuietly()
        }
    }

    private func show() {
        closeQuietly()
        guard let anchor, anchor.window != nil else { return }
        let hosting = NSHostingController(rootView: SavePopoverView(model: model))
        hosting.sizingOptions = .preferredContentSize
        let popover = NSPopover()
        popover.contentViewController = hosting
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        self.popover = popover
        // Under the button, right edge 16 in: centred on it the popover would hang past the
        // window's edge.
        guard let content = anchor.window?.contentView else {
            popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
            return
        }
        let x = content.bounds.width - 16 - SavePopoverView.width / 2
        let button = content.convert(anchor.bounds, from: anchor)
        popover.show(
            relativeTo: NSRect(x: x, y: button.minY, width: 1, height: button.height), of: content,
            preferredEdge: content.isFlipped ? .maxY : .minY)
    }

    public func closeQuietly() {
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
}
