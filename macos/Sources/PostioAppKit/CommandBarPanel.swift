import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// A borderless, non-activating child panel under the search field. It can never become key, so
/// typing and Return reach the toolbar's field, which feeds `CommandBarModel`.
@MainActor
public final class CommandBarPanel {
    private let model: CommandBarModel
    private let panel: BarPanel
    private let hosting: FirstMouseHostingView<CommandBarView>
    private weak var field: NSView?

    public var isShown: Bool { panel.isVisible }

    public var window: NSWindow { panel }

    public init(model: CommandBarModel, saveCap: @escaping () -> String?) {
        self.model = model
        panel = BarPanel(
            contentRect: NSRect(x: 0, y: 0, width: CommandBarGeometry.searchWidth, height: 200),
            styleMask: [.borderless, .nonactivatingPanel],
            backing: .buffered,
            defer: true)
        panel.isReleasedWhenClosed = false
        panel.isFloatingPanel = false
        panel.hidesOnDeactivate = false
        panel.becomesKeyOnlyIfNeeded = true
        panel.hasShadow = true
        panel.isOpaque = false
        panel.backgroundColor = .clear
        panel.animationBehavior = .none
        panel.collectionBehavior.insert(.fullScreenAuxiliary)
        panel.setAccessibilityLabel("Command bar")
        hosting = FirstMouseHostingView(rootView: CommandBarView(model: model, saveCap: saveCap()))
        hosting.sizingOptions = []
        hosting.role = .list
        hosting.setAccessibilityLabel(focusSearchWords().suggestionsLabel)
        panel.contentView = hosting
        self.saveCap = saveCap
    }

    private let saveCap: () -> String?

    var listAccessibilityRole: NSAccessibility.Role? { hosting.accessibilityRole() }
    var listAccessibilityLabel: String? { hosting.accessibilityLabel() }

    public func show(under field: NSView) {
        let field = (field as? BarSearchField)?.frameView ?? field
        guard let parent = field.window else { return }
        self.field = field
        hosting.rootView = CommandBarView(model: model, saveCap: saveCap())
        place()
        if panel.parent !== parent {
            panel.parent?.removeChildWindow(panel)
            parent.addChildWindow(panel, ordered: .above)
        }
        panel.orderFront(nil)
    }

    public func relayout() {
        guard isShown else { return }
        place()
    }

    public func hide() {
        guard panel.parent != nil || panel.isVisible else { return }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
    }

    private func place() {
        guard let field, let parent = field.window else { return }
        parent.contentView?.superview?.layoutSubtreeIfNeeded()
        let fieldOnScreen = parent.convertToScreen(field.convert(field.bounds, to: nil))
        let frame = CommandBarGeometry.frame(
            field: fieldOnScreen, window: parent.frame, height: CommandBarView.height(for: model))
        panel.setFrame(frame.integral, display: true)
    }
}

private final class BarPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// Clicks reach it at once though its window is not key: the bar's panel never is.
final class FirstMouseHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }

    /// SwiftUI's hosting view reports its own role, so one set with `setAccessibilityRole` would be
    /// ignored.
    var role: NSAccessibility.Role?
    override func accessibilityRole() -> NSAccessibility.Role? { role ?? super.accessibilityRole() }
}

/// A click into the field opens the bar, as `/` does; typing into a field with no bar under it
/// would be words the controller never hears.
public final class BarSearchField: NSSearchField {
    /// The box around the field draws the magnifier and keycap, and a cell's own would draw a
    /// second.
    override public class var cellClass: AnyClass? {
        get { BareSearchFieldCell.self }
        set {}
    }

    public var listOpen = false

    override public func accessibilityRole() -> NSAccessibility.Role? { .comboBox }
    override public func accessibilityLabel() -> String? { focusSearchWords().queryLabel }
    override public func isAccessibilityExpanded() -> Bool { listOpen }

    public var onFocus: (() -> Void)?

    public weak var frameView: NSView?

    public var textFont: NSFont = .systemFont(ofSize: 13) {
        didSet { applyFont() }
    }

    public var ghost: String? {
        didSet {
            guard ghost != oldValue else { return }
            placeGhost()
        }
    }

    public var operatorTyped = false {
        didSet {
            guard operatorTyped != oldValue else { return }
            applyFont()
        }
    }

    public var ghostHint: DropdownModel.Hint? {
        didSet {
            guard ghostHint != oldValue else { return }
            hintCap.text = ghostHint?.cap ?? ""
            hintWords.stringValue = ghostHint?.words ?? ""
            hintView.isHidden = ghostHint == nil
            placeGhost()
        }
    }

    public var operatorPrefix: String? {
        didSet {
            guard operatorPrefix != oldValue else { return }
            tintOperator()
        }
    }

    private let hintCap = KeyCapView("")
    private let hintWords: NSTextField = {
        let label = NSTextField(labelWithString: "")
        label.font = .systemFont(ofSize: 12)
        label.textColor = .secondaryLabelColor
        return label
    }()
    private lazy var hintView: NSStackView = {
        let stack = NSStackView(views: [hintCap, hintWords])
        stack.orientation = .horizontal
        stack.spacing = 4
        stack.isHidden = true
        stack.setAccessibilityElement(false)
        return stack
    }()

    private func tintOperator() {
        guard let editor = currentEditor() as? NSTextView, let layout = editor.layoutManager else { return }
        let text = editor.string as NSString
        layout.removeTemporaryAttribute(.foregroundColor, forCharacterRange: NSRange(location: 0, length: text.length))
        guard let prefix = operatorPrefix, !prefix.isEmpty else { return }
        let found = text.range(of: prefix, options: .backwards)
        guard found.location != NSNotFound else { return }
        layout.addTemporaryAttribute(.foregroundColor, value: NSColor.tertiaryLabelColor, forCharacterRange: found)
    }

    private func applyFont() {
        font = operatorTyped ? .monospacedSystemFont(ofSize: 14, weight: .regular) : textFont
        placeGhost()
    }

    private let ghostLabel: NSTextField = {
        let label = NSTextField(labelWithString: "")
        label.textColor = .tertiaryLabelColor
        label.isHidden = true
        label.setAccessibilityElement(false)
        return label
    }()

    override public func becomeFirstResponder() -> Bool {
        let became = super.becomeFirstResponder()
        if became { onFocus?() }
        return became
    }

    override public func layout() {
        super.layout()
        placeGhost()
    }

    override public func textDidChange(_ notification: Notification) {
        super.textDidChange(notification)
        tintOperator()
        placeGhost()
    }

    private func placeGhost() {
        if ghostLabel.superview == nil { addSubview(ghostLabel) }
        if hintView.superview == nil { addSubview(hintView) }
        let hintSize = hintView.fittingSize
        hintView.frame = NSRect(
            x: bounds.maxX - hintSize.width, y: (bounds.height - hintSize.height) / 2,
            width: hintSize.width, height: hintSize.height)
        guard let ghost, !ghost.isEmpty, let font else {
            ghostLabel.isHidden = true
            return
        }
        let typed = currentEditor()?.string ?? stringValue
        let text = (cell as? NSSearchFieldCell)?.searchTextRect(forBounds: bounds) ?? bounds
        let width = (typed as NSString).size(withAttributes: [.font: font]).width
        ghostLabel.font = font
        ghostLabel.stringValue = ghost
        ghostLabel.sizeToFit()
        let size = ghostLabel.frame.size
        // The ghost follows the last glyph where the field editor laid it out; without an editor,
        // two points in (line fragment padding). The label's text is inset in its frame, which is
        // taken back so "at" and "las" read as one word.
        let end = lastGlyphEnd() ?? (text.minX + 2 + width)
        let inset = ghostLabel.cell?.titleRect(forBounds: ghostLabel.bounds).minX ?? 0
        let x = end - inset
        let room = (hintView.isHidden ? text.maxX : hintView.frame.minX - 8) - x
        ghostLabel.frame = NSRect(
            x: x, y: (bounds.height - size.height) / 2,
            width: min(size.width, max(room, 0)), height: size.height)
        ghostLabel.isHidden = false
    }

    private func lastGlyphEnd() -> CGFloat? {
        guard let editor = currentEditor() as? NSTextView,
            let layout = editor.layoutManager, let container = editor.textContainer,
            layout.numberOfGlyphs > 0
        else { return nil }
        let last = layout.boundingRect(
            forGlyphRange: NSRange(location: layout.numberOfGlyphs - 1, length: 1), in: container)
        let x = last.maxX + editor.textContainerOrigin.x
        return editor.convert(NSPoint(x: x, y: 0), to: self).x
    }
}

final class BareSearchFieldCell: NSSearchFieldCell {
    override func searchButtonRect(forBounds rect: NSRect) -> NSRect { .zero }
    override func cancelButtonRect(forBounds rect: NSRect) -> NSRect { .zero }
    override func searchTextRect(forBounds rect: NSRect) -> NSRect { rect }
    override func drawingRect(forBounds rect: NSRect) -> NSRect {
        let drawn = super.drawingRect(forBounds: rect)
        return NSRect(x: rect.minX, y: drawn.minY, width: rect.width, height: drawn.height)
    }
}
