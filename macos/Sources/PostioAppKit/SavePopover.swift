import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The Save popover's presenter (specs/010-focus-search T100; design §3.9,
/// screen 12): an `NSPopover` with its arrow, hung from the toolbar's Save
/// search button, holding `SavePopoverView`. It opens and closes when
/// `SavePopoverModel` says the controller did; a close the toolkit made --
/// a click away -- writes nothing.
@MainActor
public final class SavePopover: NSObject, NSPopoverDelegate {
    private let model: SavePopoverModel
    private var popover: NSPopover?
    private var closingQuietly = false
    /// The view it hangs from: the Save search button, while the results'
    /// toolbar is up.
    public weak var anchor: NSView?

    public init(model: SavePopoverModel) {
        self.model = model
        super.init()
        model.dismiss = { [weak self] in self?.closeQuietly() }
    }

    /// Whether it is up.
    public var isShown: Bool { popover?.isShown == true }

    /// Show what `change` says.
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
        // Under the button, its right edge 16 in from the window's (screen
        // 12): centred on the button it would hang past the window's edge,
        // so it hangs from a point level with the button, further in.
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

    /// Take it down without telling anyone: the controller, or the model's
    /// own Cancel, already knows.
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
