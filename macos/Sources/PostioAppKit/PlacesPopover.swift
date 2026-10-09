import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The folders and labels popover (specs/009-focus-macos T086; screen 10;
/// contracts/mac-surfaces.md, "Popovers").
///
/// An `NSPopover` anchored to the header strip's Inbox ▾, opened on
/// `FocusOpenPlaces` (`g o`, or a click on the button). A filter field on
/// top keeps the keyboard: what is typed narrows the places
/// (`focusPlaces(filter)`), ↑/↓ move the highlight and Return opens the
/// highlighted place (`focusOpenPlace`). Below it is `PlacesView`. Escape,
/// a click outside, or opening a place closes it, and `closed` is called
/// once, for the keyboard to go home.
@MainActor
public final class PlacesPopover: NSObject, NSPopoverDelegate, NSSearchFieldDelegate {
    private let model: PlacesModel
    private let closed: () -> Void
    private let popover = NSPopover()
    private let field = NSSearchField()
    private let hosting: NSHostingView<PlacesView>
    private let controller = NSViewController()

    /// The view it hangs from: the strip's Inbox ▾ (`PlacesAnchor`).
    public weak var anchor: NSView?

    /// Whether it is on screen.
    public var isShown: Bool { popover.isShown }

    /// The popover's filter field, for a replay to type into.
    public var filterField: NSSearchField { field }

    public init(model: PlacesModel, closed: @escaping () -> Void) {
        self.model = model
        self.closed = closed
        hosting = NSHostingView(rootView: PlacesView(model: model, open: { _ in }))
        super.init()
        hosting.rootView = PlacesView(model: model) { [weak self] token in
            guard let self, self.model.isOpen else { return }
            self.model.open(token)
            self.closeOpened()
        }
        hosting.sizingOptions = []
        field.delegate = self
        field.sendsSearchStringImmediately = true
        field.focusRingType = .none
        field.controlSize = .large
        field.font = .systemFont(ofSize: 14)

        let container = NSView()
        field.translatesAutoresizingMaskIntoConstraints = false
        hosting.translatesAutoresizingMaskIntoConstraints = false
        container.addSubview(field)
        container.addSubview(hosting)
        let inset: CGFloat = 10
        NSLayoutConstraint.activate([
            field.topAnchor.constraint(equalTo: container.topAnchor, constant: inset),
            field.leadingAnchor.constraint(equalTo: container.leadingAnchor, constant: inset),
            field.trailingAnchor.constraint(equalTo: container.trailingAnchor, constant: -inset),
            hosting.topAnchor.constraint(equalTo: container.topAnchor, constant: PlacesView.Metrics.field),
            hosting.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            hosting.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        controller.view = container
        popover.contentViewController = controller
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
    }

    /// Show it under the anchor, the filter empty and holding the keyboard.
    public func show() {
        guard let anchor, anchor.window != nil else { return }
        field.stringValue = ""
        field.placeholderString = model.placeholder
        resize()
        if !popover.isShown {
            popover.show(relativeTo: anchor.bounds, of: anchor, preferredEdge: .minY)
        }
        popover.contentViewController?.view.window?.makeFirstResponder(field)
    }

    /// The places were read again: the size follows them.
    public func reload() {
        guard popover.isShown else { return }
        resize()
    }

    /// Close it, however it was asked to close.
    public func close() {
        guard popover.isShown else {
            if model.close() { closed() }
            return
        }
        popover.performClose(nil)
    }

    /// Type `text` at the end of the filter, as a replay does.
    public func type(_ text: String) {
        field.stringValue += text
        model.filterChanged(field.stringValue)
        resize()
    }

    private func resize() {
        let screen = anchor?.window?.screen?.visibleFrame.height ?? 900
        let tallest = max(screen - 120, 240)
        let height = min(PlacesView.Metrics.field + PlacesView.height(for: model), tallest)
        popover.contentSize = NSSize(width: PlacesView.Metrics.width, height: height)
    }

    // MARK: NSPopoverDelegate

    public func popoverDidClose(_ notification: Notification) {
        if model.close() { closed() }
    }

    // MARK: NSSearchFieldDelegate

    public func controlTextDidChange(_ obj: Notification) {
        model.filterChanged(field.stringValue)
        resize()
    }

    public func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveUp(_:)):
            model.move(by: -1)
        case #selector(NSResponder.moveDown(_:)):
            model.move(by: 1)
        case #selector(NSResponder.insertNewline(_:)):
            openHighlighted()
        case #selector(NSResponder.cancelOperation(_:)):
            close()
        default:
            return false
        }
        return true
    }

    /// Return: go to the highlighted place, and close.
    public func openHighlighted() {
        guard model.openHighlighted() else { return }
        closeOpened()
    }

    /// A place was opened from the list (a click), which closed the model;
    /// the popover follows.
    func closeOpened() {
        popover.performClose(nil)
        closed()
    }
}

/// The view the folders popover hangs from: put behind the strip's
/// Inbox ▾ (`HeaderStrip`'s `placeAnchor`), it reports itself once it is
/// in a window.
public struct PlacesAnchor: NSViewRepresentable {
    let found: (NSView) -> Void

    public init(found: @escaping (NSView) -> Void) {
        self.found = found
    }

    public func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        let found = found
        DispatchQueue.main.async { found(view) }
        return view
    }

    public func updateNSView(_: NSView, context _: Context) {}
}
