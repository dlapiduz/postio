import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The known-sender popover's presenter (specs/011-focus-contacts S6,
/// handoff screen 04): an `NSPopover` with its arrow, hung from the
/// sender's name, holding `ContactPopoverView`. It opens, redraws and
/// closes when `ContactPopoverModel` says the controller did; a close the
/// toolkit made -- a click away -- is told to the controller by whoever
/// owns this, through `onToolkitClose`.
@MainActor
public final class ContactPopover: NSObject, NSPopoverDelegate {
    private let model: ContactPopoverModel
    private var popover: NSPopover?
    private var closingQuietly = false

    /// Where it hangs from: the view and the rectangle in it, the sender's
    /// name. `nil` when there is nothing to hang it from.
    public var anchor: (() -> (view: NSView, rect: NSRect)?)?
    /// A click away took it down.
    public var onToolkitClose: (() -> Void)?

    public init(model: ContactPopoverModel) {
        self.model = model
        super.init()
    }

    /// About how tall the tallest popover is (the new-sender one), which
    /// decides whether it fits below a name.
    static let expectedHeight: CGFloat = 380

    /// Whether it is up.
    public var isShown: Bool { popover?.isShown == true }

    /// Show what `change` says.
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
        // Below the name, unless there is not room for it there: then above,
        // as handoff screen 05 draws it for the row near the foot. (AppKit
        // would flip an overflowing popover itself; saying so keeps the
        // choice ours and the same on every screen.)
        let below: NSRectEdge = view.isFlipped ? .maxY : .minY
        let above: NSRectEdge = view.isFlipped ? .minY : .maxY
        var edge = below
        if let window = view.window, let screen = window.screen {
            let onScreen = window.convertToScreen(view.convert(rect, to: nil))
            if onScreen.minY - screen.visibleFrame.minY < Self.expectedHeight { edge = above }
        }
        popover.show(relativeTo: rect, of: view, preferredEdge: edge)
    }

    /// Take it down without telling anyone: the controller already knows.
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
