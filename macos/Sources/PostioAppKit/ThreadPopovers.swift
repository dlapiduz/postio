import AppKit
import PostioFFI
import PostioKit
import SwiftUI

@MainActor
public final class ThreadMenuPopover: NSObject, NSPopoverDelegate {
    private let model: ThreadMenuModel
    private let popover = NSPopover()
    private let keyView = ThreadMenuKeyView()
    private let hosting: NSHostingController<ThreadMenuView>
    private let onToolkitClose: () -> Void

    public var isShown: Bool { popover.isShown }

    public init(model: ThreadMenuModel, onToolkitClose: @escaping () -> Void) {
        self.model = model
        self.onToolkitClose = onToolkitClose
        hosting = NSHostingController(rootView: ThreadMenuView(model: model))
        super.init()
        hosting.sizingOptions = []
        keyView.model = model
        let container = NSView()
        let content = hosting.view
        for view in [keyView, content] {
            view.translatesAutoresizingMaskIntoConstraints = false
            container.addSubview(view)
            NSLayoutConstraint.activate([
                view.topAnchor.constraint(equalTo: container.topAnchor),
                view.leadingAnchor.constraint(equalTo: container.leadingAnchor),
                view.trailingAnchor.constraint(equalTo: container.trailingAnchor),
                view.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            ])
        }
        let controller = NSViewController()
        controller.view = container
        popover.contentViewController = controller
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
    }

    public func show(relativeTo rect: NSRect, of view: NSView) {
        guard view.window != nil else { return }
        hosting.view.layoutSubtreeIfNeeded()
        let fitting = hosting.sizeThatFits(in: NSSize(width: ThreadMenuMetrics.width, height: 10_000))
        popover.contentSize = NSSize(width: ThreadMenuMetrics.width, height: ceil(max(fitting.height, 120)))
        if popover.isShown { popover.positioningRect = rect } else {
            popover.show(relativeTo: rect, of: view, preferredEdge: view.isFlipped ? .maxY : .minY)
        }
        popover.contentViewController?.view.window?.makeFirstResponder(keyView)
    }

    public func close() {
        guard popover.isShown else { return }
        popover.performClose(nil)
    }

    public func popoverDidClose(_ notification: Notification) {
        if model.closedByToolkit() { onToolkitClose() }
    }
}

final class ThreadMenuKeyView: NSView {
    weak var model: ThreadMenuModel?

    override var acceptsFirstResponder: Bool { true }

    override func keyDown(with event: NSEvent) {
        interpretKeyEvents([event])
    }

    override func moveUp(_ sender: Any?) { model?.move(by: -1) }
    override func moveDown(_ sender: Any?) { model?.move(by: 1) }
    override func insertNewline(_ sender: Any?) { model?.confirm() }
    override func insertText(_ insertString: Any) {}
    override func doCommand(by selector: Selector) {}
}

@MainActor
public final class ThreadOutlinePopover: NSObject, NSPopoverDelegate, NSSearchFieldDelegate {
    private let model: ThreadOutlineModel
    private let popover = NSPopover()
    private let filter = NSSearchField()
    private let hosting: NSHostingController<ThreadOutlineView<PickerFieldHost>>
    private let onToolkitClose: () -> Void

    public var isShown: Bool { popover.isShown }

    public init(model: ThreadOutlineModel, onToolkitClose: @escaping () -> Void) {
        self.model = model
        self.onToolkitClose = onToolkitClose
        let filter = filter
        hosting = NSHostingController(rootView: ThreadOutlineView(model: model) {
            PickerFieldHost(field: filter)
        })
        super.init()
        hosting.sizingOptions = []
        filter.delegate = self
        filter.sendsSearchStringImmediately = true
        filter.focusRingType = .none
        filter.controlSize = .large
        filter.font = .systemFont(ofSize: 13.5)
        filter.placeholderString = "Filter this thread"
        popover.contentViewController = hosting
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
    }

    public func show(relativeTo rect: NSRect, of view: NSView) {
        guard view.window != nil else { return }
        sync()
        if popover.isShown { popover.positioningRect = rect } else {
            popover.show(relativeTo: rect, of: view, preferredEdge: view.isFlipped ? .maxY : .minY)
        }
        focusFilter()
    }

    public func reload() {
        guard popover.isShown else { return }
        sync()
    }

    public func focusFilter() {
        guard popover.isShown else { return }
        filter.window?.makeFirstResponder(filter)
    }

    public func close() {
        guard popover.isShown else { return }
        popover.performClose(nil)
    }

    public func type(_ text: String) {
        if let editor = filter.currentEditor() as? NSTextView {
            editor.insertText(text, replacementRange: editor.selectedRange())
        } else {
            filter.stringValue += text
            model.typed(filter.stringValue)
        }
    }

    private func sync() {
        if let text = model.view?.filter, filter.stringValue != text { filter.stringValue = text }
        measure()
        DispatchQueue.main.async { [weak self] in self?.measure() }
    }

    private func measure() {
        hosting.view.layoutSubtreeIfNeeded()
        let fitting = hosting.sizeThatFits(in: NSSize(width: ThreadOutlineMetrics.width, height: 10_000))
        let height = fitting.height > 0 ? ceil(fitting.height) : 320
        popover.contentSize = NSSize(width: ThreadOutlineMetrics.width, height: height)
    }

    public func popoverDidClose(_ notification: Notification) {
        if model.closedByToolkit() { onToolkitClose() }
    }

    public func controlTextDidChange(_ obj: Notification) {
        guard let field = obj.object as? NSTextField else { return }
        model.typed(field.stringValue)
    }

    public func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveUp(_:)): model.move(by: -1)
        case #selector(NSResponder.moveDown(_:)): model.move(by: 1)
        case #selector(NSResponder.insertNewline(_:)): model.show()
        case #selector(NSResponder.cancelOperation(_:)): break
        default: return false
        }
        return true
    }
}
