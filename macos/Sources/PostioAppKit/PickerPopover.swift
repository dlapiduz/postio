import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The pickers at the row: snooze, remind, label and move
/// (specs/009-focus-macos T092; screens 11 to 14; contracts/mac-surfaces.md,
/// "Popovers").
///
/// An `NSPopover` with `.transient` behaviour, hung from what the
/// controller names (`FocusOpenPicker`'s anchor): under the cursor's row at
/// the subject column, or under the message window's action button.
/// `PickerView` draws it around an AppKit field this owns:
///
/// - **a filter** (label, move) above the rows, holding the keyboard from
///   the start. While it is empty the resolver gives a bare digit or Space
///   to the picker; anything else is typing, and every change is
///   `focusPickerTyped`.
/// - **a date field** (snooze, remind) under the rows, which takes the
///   keyboard only when the controller says (`FocusPickerField`, Tab).
///   Until then the keyboard is on the popover's `PickerKeyView`, so the
///   number keys, Space, Return and Tab resolve as the picker's commands.
///
/// ↑/↓ and Return in either field, and ↑/↓ on the key view, go to
/// `PickerModel`, which keeps the highlight. Escape resolves to Back before
/// the field sees it.
///
/// Closing: the controller closes it (`FocusCloseSurface(.picker)`), and
/// `close()` is not reported back -- the controller took it off its stack
/// itself (T085's rule for the bar). Any other close -- a click outside,
/// the popover dismissing itself -- is the toolkit's, and `closedByToolkit`
/// is called once, for the engine to say `focusSurfaceClosed(.picker)`.
@MainActor
public final class PickerPopover: NSObject, NSPopoverDelegate, NSSearchFieldDelegate {
    private let model: PickerModel
    private let closedByToolkit: () -> Void
    private let popover = NSPopover()
    private let keyView = PickerKeyView()
    private let filter: NSSearchField
    private let date: NSTextField
    private let hosting: NSHostingView<PickerView<PickerFieldHost>>
    private let controller = NSViewController()

    /// Whether it is on screen.
    public var isShown: Bool { popover.isShown }

    public init(model: PickerModel, closedByToolkit: @escaping () -> Void) {
        self.model = model
        self.closedByToolkit = closedByToolkit
        let filter = NSSearchField()
        let date = NSTextField()
        self.filter = filter
        self.date = date
        hosting = NSHostingView(rootView: PickerView(model: model) { kind in
            PickerFieldHost(field: kind == .filter ? filter : date)
        })
        super.init()
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
        keyView.translatesAutoresizingMaskIntoConstraints = false
        hosting.translatesAutoresizingMaskIntoConstraints = false
        let container = NSView()
        container.addSubview(keyView)
        container.addSubview(hosting)
        NSLayoutConstraint.activate([
            keyView.topAnchor.constraint(equalTo: container.topAnchor),
            keyView.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            keyView.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            keyView.bottomAnchor.constraint(equalTo: container.bottomAnchor),
            hosting.topAnchor.constraint(equalTo: container.topAnchor),
            hosting.leadingAnchor.constraint(equalTo: container.leadingAnchor),
            hosting.trailingAnchor.constraint(equalTo: container.trailingAnchor),
            hosting.bottomAnchor.constraint(equalTo: container.bottomAnchor),
        ])
        controller.view = container
        popover.contentViewController = controller
        popover.behavior = .transient
        // Absent rather than slow (PRODUCT.md §18): the picker is there
        // when the key is.
        popover.animates = false
        popover.delegate = self
    }

    /// Show the picker the model holds, hung from `rect` in `view`, with
    /// the keyboard where its kind wants it: the filter, or the key view
    /// until Tab.
    public func show(relativeTo rect: NSRect, of view: NSView) {
        guard view.window != nil else { return }
        let field = model.field == .filter ? filter as NSTextField : date
        field.stringValue = model.typed
        field.placeholderString = model.placeholder
        resize()
        if popover.isShown {
            popover.positioningRect = rect
        } else {
            // Below the row: in a flipped view (the table, SwiftUI's
            // hosting views) that is the larger y.
            popover.show(relativeTo: rect, of: view, preferredEdge: view.isFlipped ? .maxY : .minY)
        }
        let window = popover.contentViewController?.view.window
        window?.makeFirstResponder(model.filters ? filter : keyView)
    }

    /// The picker was drawn again: the field follows the controller's words
    /// when they differ from what it holds, and the size follows the rows.
    public func reload() {
        guard popover.isShown else { return }
        let field = model.field == .filter ? filter as NSTextField : date
        if field.stringValue != model.typed { field.stringValue = model.typed }
        resize()
    }

    /// `Tab` in a date picker: the field takes the keyboard.
    public func focusField() {
        guard popover.isShown else { return }
        date.window?.makeFirstResponder(date)
        date.currentEditor()?.selectedRange = NSRange(location: (date.stringValue as NSString).length, length: 0)
        resize()
    }

    /// Close it because the controller did. Not reported back.
    public func close() {
        guard popover.isShown else { return }
        popover.performClose(nil)
    }

    /// Type `text` at the end of whichever field it holds, as a replay does.
    public func type(_ text: String) {
        let field = model.field == .filter ? filter as NSTextField : date
        if let editor = field.currentEditor() as? NSTextView {
            editor.insertText(text, replacementRange: editor.selectedRange())
        } else {
            field.stringValue += text
            model.typed(field.stringValue)
        }
    }

    private func resize() {
        let fitting = hosting.fittingSize
        let height = fitting.height > 0 ? fitting.height : 240
        popover.contentSize = NSSize(width: PickerMetrics.width, height: height)
    }

    // MARK: NSPopoverDelegate

    public func popoverDidClose(_ notification: Notification) {
        // Closed by the controller: the model has already forgotten it.
        if model.closedByToolkit() { closedByToolkit() }
    }

    // MARK: NSTextFieldDelegate

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
            // A fallback: the key monitor resolves Escape to Back first.
            model.back()
        case #selector(NSResponder.insertTab(_:)), #selector(NSResponder.insertBacktab(_:)):
            // The field keeps the keyboard; there is nowhere else in the
            // picker for Tab to go.
            break
        default:
            return false
        }
        return true
    }
}

/// What holds the keyboard in a date picker until Tab: the key monitor
/// resolves the number keys, Space, Return and Tab as the picker's
/// commands, and the arrows, which no binding takes in a picker, reach
/// this and move the highlight.
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
    // Anything typed here that no binding took is not a picker's: say
    // nothing rather than beep at every letter.
    override func insertText(_ insertString: Any) {}
    override func doCommand(by selector: Selector) {}
}

/// The picker's AppKit field, as SwiftUI places it in `PickerView`.
public struct PickerFieldHost: NSViewRepresentable {
    let field: NSTextField

    public func makeNSView(context: Context) -> NSTextField {
        field
    }

    public func updateNSView(_: NSTextField, context _: Context) {}
}
