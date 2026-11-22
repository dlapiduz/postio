import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// A close the toolkit made (a click away) is told to the controller by the owner through
/// `onToolkitClose`.
@MainActor
public final class ContactPopover: NSObject, NSPopoverDelegate {
    private let model: ContactPopoverModel
    private var popover: NSPopover?
    private var closingQuietly = false

    public var anchor: (() -> (view: NSView, rect: NSRect)?)?
    public var onToolkitClose: (() -> Void)?

    public init(model: ContactPopoverModel) {
        self.model = model
        super.init()
    }

    static let expectedHeight: CGFloat = 380

    public var isShown: Bool { popover?.isShown == true }

    public func apply(_ change: ContactPopoverModel.Change) {
        switch change {
        case .open: show()
        case .redraw: break
        case .close: closeQuietly()
        }
    }

    private func show() {
        closeQuietly()
        guard let (view, rect) = anchor?(), view.window != nil else { return }
        let hosting = NSHostingController(rootView: ContactPopoverView(model: model))
        hosting.sizingOptions = .preferredContentSize
        let popover = NSPopover()
        popover.contentViewController = hosting
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
        self.popover = popover
        // Below the name unless there is no room, then above: AppKit would flip it itself, but this
        // keeps the choice ours and the same on every screen.
        let below: NSRectEdge = view.isFlipped ? .maxY : .minY
        let above: NSRectEdge = view.isFlipped ? .minY : .maxY
        var edge = below
        if let window = view.window, let screen = window.screen {
            let onScreen = window.convertToScreen(view.convert(rect, to: nil))
            if onScreen.minY - screen.visibleFrame.minY < Self.expectedHeight { edge = above }
        }
        popover.show(relativeTo: rect, of: view, preferredEdge: edge)
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
        onToolkitClose?()
    }
}
