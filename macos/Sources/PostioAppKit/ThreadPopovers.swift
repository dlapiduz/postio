import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// Thread ▾'s menu as an `NSPopover` (spec 013 US5, T066; screen 04): hung
/// from the button in the thread bar, 340 wide. The controller opens and
/// closes it (`FocusThreadMenu`, `FocusCloseSurface(.threadMenu)`); a close
/// the toolkit made -- a click away -- is reported through `onToolkitClose`
/// for the engine to say `focusSurfaceClosed(.threadMenu)`.
///
/// ↑/↓ and Return reach `ThreadMenuModel` through a key view that holds the
/// keyboard; the entries' own keys resolve as the Reader's commands.
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
        // Absent rather than slow: the menu is there when the key is.
        popover.animates = false
        popover.delegate = self
    }

    /// Show the menu the model holds, under `rect` in `view`.
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

    /// Close it because the controller did. Not reported back.
    public func close() {
        guard popover.isShown else { return }
        popover.performClose(nil)
    }

    public func popoverDidClose(_ notification: Notification) {
        if model.closedByToolkit() { onToolkitClose() }
    }
}

/// What holds the keyboard in the menu: the arrows move the highlight and
/// Return chooses; every other key was the resolver's before it got here.
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

/// The outline as an `NSPopover` (screen 05): 440 wide, from the same
/// button. The filter is an `NSSearchField`, which holds the keyboard from
/// the start; ↑/↓ and Return in it go to `ThreadOutlineModel`, and every
/// change of its text is `focusOutlineTyped`. `/` puts the keyboard back in
/// it (`FocusOutlineFocusFilter`).
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

    /// Show the outline the model holds, under `rect` in `view`, with the
    /// keyboard in the filter.
    public func show(relativeTo rect: NSRect, of view: NSView) {
        guard view.window != nil else { return }
        sync()
        if popover.isShown { popover.positioningRect = rect } else {
            popover.show(relativeTo: rect, of: view, preferredEdge: view.isFlipped ? .maxY : .minY)
        }
        focusFilter()
    }

    /// It was drawn again: the field follows the controller's words when
    /// they differ from what it holds, and the size follows the rows.
    public func reload() {
        guard popover.isShown else { return }
        sync()
    }

    /// `/`: the keyboard goes to the filter.
    public func focusFilter() {
        guard popover.isShown else { return }
        filter.window?.makeFirstResponder(filter)
    }

    /// Close it because the controller did. Not reported back.
    public func close() {
        guard popover.isShown else { return }
        popover.performClose(nil)
    }

    /// Type `text` at the end of the filter, as a replay does.
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

    // MARK: NSPopoverDelegate

    public func popoverDidClose(_ notification: Notification) {
        if model.closedByToolkit() { onToolkitClose() }
    }

    // MARK: NSSearchFieldDelegate

    public func controlTextDidChange(_ obj: Notification) {
        guard let field = obj.object as? NSTextField else { return }
        model.typed(field.stringValue)
    }

    public func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        switch selector {
        case #selector(NSResponder.moveUp(_:)): model.move(by: -1)
        case #selector(NSResponder.moveDown(_:)): model.move(by: 1)
        case #selector(NSResponder.insertNewline(_:)): model.show()
        // Escape resolves to Back before the field sees it; a fallback.
        case #selector(NSResponder.cancelOperation(_:)): break
        default: return false
        }
        return true
    }
}
