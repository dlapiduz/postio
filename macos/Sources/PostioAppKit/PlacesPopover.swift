import AppKit
import PostioFFI
import PostioKit
import SwiftUI

@MainActor
public final class PlacesPopover: NSObject, NSPopoverDelegate, NSSearchFieldDelegate {
    private let model: PlacesModel
    private let closed: () -> Void
    private let popover = NSPopover()
    private let field = NSSearchField()
    private let hosting: NSHostingView<PlacesView>
    private let controller = NSViewController()

    public weak var anchor: NSView?

    public var isShown: Bool { popover.isShown }

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

    public func show() {
        guard let anchor, anchor.window != nil else { return }
        field.stringValue = ""
        field.placeholderString = model.placeholder
        resize()
        if !popover.isShown {
            // No arrow: it would point at the middle of the rect rather than the button. From the
            // window's
            // content view, since AppKit clips the rect to the view it is given.
            hideArrow()
            let host = anchor.window?.contentView ?? anchor
            let button = host.convert(anchor.bounds, from: anchor)
            popover.show(
                relativeTo: PlacesView.Metrics.positioningRect(button: button), of: host,
                preferredEdge: host.isFlipped ? .maxY : .minY)
        }
        popover.contentViewController?.view.window?.makeFirstResponder(field)
    }

    /// AppKit has no public switch before macOS 15's `hasFullSizeContent`; `shouldHideAnchor` is
    /// the long-standing one.
    private func hideArrow() {
        let key = "shouldHideAnchor"
        if popover.responds(to: NSSelectorFromString("setShouldHideAnchor:")) {
            popover.setValue(true, forKey: key)
        }
    }

    public func reload() {
        guard popover.isShown else { return }
        resize()
    }

    public func close() {
        guard popover.isShown else {
            if model.close() { closed() }
            return
        }
        popover.performClose(nil)
    }

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

    public func popoverDidClose(_ notification: Notification) {
        if model.close() { closed() }
    }

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

    public func openHighlighted() {
        guard model.openHighlighted() else { return }
        closeOpened()
    }

    func closeOpened() {
        popover.performClose(nil)
        closed()
    }
}

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
