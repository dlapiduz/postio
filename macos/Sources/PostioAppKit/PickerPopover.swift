import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// An `NSPopover` with `.transient` behaviour. The filter (label, move) holds the keyboard from the
/// start; the date field (snooze, remind) takes it only when the controller says (Tab), until then
/// `PickerKeyView` has it so number keys, Space, Return and Tab resolve as the picker's commands. A
/// close by the controller is not reported back; any other is the toolkit's, and `closedByToolkit`
/// is called once.
@MainActor
public final class PickerPopover: NSObject, NSPopoverDelegate, NSSearchFieldDelegate {
    private let model: PickerModel
    private let closedByToolkit: () -> Void
    private let popover = NSPopover()
    private let keyView = PickerKeyView()
    private let filter: NSSearchField
    private let date: NSTextField
    private let hosting: NSHostingController<PickerView<PickerFieldHost>>
    private let controller = NSViewController()

    public var isShown: Bool { popover.isShown }

    public init(model: PickerModel, closedByToolkit: @escaping () -> Void) {
        self.model = model
        self.closedByToolkit = closedByToolkit
        let filter = NSSearchField()
        let date = NSTextField()
        self.filter = filter
        self.date = date
        hosting = NSHostingController(rootView: PickerView(model: model) { kind in
            PickerFieldHost(field: kind == .filter ? filter : date)
        })
        super.init()
        // The popover's size is set from `sizeThatFits`, never left to the
        // content's constraints.
        hosting.sizingOptions = []

        filter.delegate = self
        filter.sendsSearchStringImmediately = true
        filter.focusRingType = .none
        filter.controlSize = .large
        filter.font = .systemFont(ofSize: 14)

        date.delegate = self
        date.isBordered = false
        date.isBezeled = false
        date.drawsBackground = false
        date.focusRingType = .none
        date.font = .systemFont(ofSize: 14)
        date.cell?.isScrollable = true
        date.cell?.wraps = false

        keyView.model = model
        let content = hosting.view
        keyView.translatesAutoresizingMaskIntoConstraints = false
        content.translatesAutoresizingMaskIntoConstraints = false
        let container = NSView()
        container.addSubview(keyView)
        container.addSubview(content)
        NSLayoutConstraint.activate([
            keyView.topAnchor.constraint(equalTo: container.topAnchor),
            keyView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            keyView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            keyView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            content.topAnchor.constraint(equalTo: container.topAnchor),
            content.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            content.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            content.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        controller.view = container
        popover.contentViewController = controller
        popover.behavior = .transient
        popover.animates = false
        popover.delegate = self
    }

    public func show(relativeTo rect: NSRect, of view: NSView) {
        guard view.window != nil else { return }
        let field = model.field == .filter ? filter as NSTextField : date
        field.stringValue = model.typed
        field.placeholderString = model.placeholder
        resize()
        if popover.isShown {
            popover.positioningRect = rect
        } else {
            popover.show(relativeTo: rect, of: view, preferredEdge: view.isFlipped ? .maxY : .minY)
        }
        let window = popover.contentViewController?.view.window
        window?.makeFirstResponder(model.filters ? filter : keyView)
    }

    public func reload() {
        guard popover.isShown else { return }
        let field = model.field == .filter ? filter as NSTextField : date
        if field.stringValue != model.typed { field.stringValue = model.typed }
        resize()
    }

    public func focusField() {
        guard popover.isShown else { return }
        date.window?.makeFirstResponder(date)
        date.currentEditor()?.selectedRange = NSRange(location: (date.stringValue as NSString).length, length: 0)
        resize()
    }

    public func close() {
        guard popover.isShown else { return }
        popover.performClose(nil)
    }

    public func type(_ text: String) {
        let field = model.field == .filter ? filter as NSTextField : date
        if let editor = field.currentEditor() as? NSTextView {
            editor.insertText(text, replacementRange: editor.selectedRange())
        } else {
            field.stringValue += text
            model.typed(field.stringValue)
        }
    }

    /// The hosting controller answers `sizeThatFits` from the view it last updated, so a measure in
    /// the same turn as a redraw sees the old rows.
    private func resize() {
        measure()
        DispatchQueue.main.async { [weak self] in self?.measure() }
    }

    private func measure() {
        hosting.view.layoutSubtreeIfNeeded()
        let fitting = hosting.sizeThatFits(in: NSSize(width: PickerMetrics.width, height: 10_000))
        let height = fitting.height > 0 ? ceil(fitting.height) : 240
        popover.contentSize = NSSize(width: PickerMetrics.width, height: height)
    }

    public func popoverDidClose(_ notification: Notification) {
        if model.closedByToolkit() { closedByToolkit() }
    }

    public func controlTextDidChange(_ obj: Notification) {
        guard let field = obj.object as? NSTextField else { return }
        model.typed(field.stringValue)
    }

    public func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveUp(_:)):
            model.move(by: -1)
        case #selector(NSResponder.moveDown(_:)):
            model.move(by: 1)
        case #selector(NSResponder.insertNewline(_:)):
            model.confirm()
        case #selector(NSResponder.cancelOperation(_:)):
            model.back()
        case #selector(NSResponder.insertTab(_:)), #selector(NSResponder.insertBacktab(_:)):
            break
        default:
            return false
        }
        return true
    }
}

final class PickerKeyView: NSView {
    weak var model: PickerModel?

    override var acceptsFirstResponder: Bool { true }

    override func keyDown(with event: NSEvent) {
        interpretKeyEvents([event])
    }

    override func moveUp(_ sender: Any?) { model?.move(by: -1) }
    override func moveDown(_ sender: Any?) { model?.move(by: 1) }
    override func insertNewline(_ sender: Any?) { model?.confirm() }
    override func cancelOperation(_ sender: Any?) { model?.back() }
    // Anything typed here that no binding took says nothing rather than beeping at every letter.
    override func insertText(_ insertString: Any) {}
    override func doCommand(by selector: Selector) {}
}

public struct PickerFieldHost: NSViewRepresentable {
    let field: NSTextField

    public func makeNSView(context: Context) -> NSTextField {
        field
    }

    public func updateNSView(_: NSTextField, context _: Context) {}
}
