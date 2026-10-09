import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The command bar on the Mac (specs/009-focus-macos T085; screens 07 to
/// 09; contracts/mac-surfaces.md, "Command bar").
///
/// A borderless, non-activating child panel hung 6 below the search field,
/// with its left edge and width: the field grows to 860 while the bar is up
/// (specs/010-focus-search T054). Nothing behind it is dimmed. The keyboard stays
/// in the toolbar's field: the panel can never become key, so typing,
/// ↑/↓, Return, Tab and Escape all arrive at the field and go to
/// `CommandBarModel` from there. Its content is `CommandBarView`.
///
/// It opens and closes when the controller says (`FocusOpenBar`,
/// `FocusCloseSurface(.bar)`); this only puts it on screen and takes it
/// off.
@MainActor
public final class CommandBarPanel {
    private let model: CommandBarModel
    private let panel: BarPanel
    private let hosting: FirstMouseHostingView<CommandBarView>
    private weak var field: NSView?

    /// Whether it is on screen.
    public var isShown: Bool { panel.isVisible }

    /// The panel itself, for a test or a capture to find.
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
        panel.contentView = hosting
        self.saveCap = saveCap
    }

    private let saveCap: () -> String?

    /// Put the panel under `field`, a child of the field's window, or move
    /// it there and resize it for what the model holds now.
    public func show(under field: NSView) {
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

    /// The lines changed: the panel's height follows them.
    public func relayout() {
        guard isShown else { return }
        place()
    }

    /// Take it off screen.
    public func hide() {
        guard panel.parent != nil || panel.isVisible else { return }
        panel.parent?.removeChildWindow(panel)
        panel.orderOut(nil)
    }

    private func place() {
        guard let field, let parent = field.window else { return }
        // The toolbar has just grown the field: lay it out before reading it.
        parent.contentView?.superview?.layoutSubtreeIfNeeded()
        let fieldOnScreen = parent.convertToScreen(field.convert(field.bounds, to: nil))
        let frame = CommandBarGeometry.frame(
            field: fieldOnScreen, window: parent.frame, height: CommandBarView.height(for: model))
        panel.setFrame(frame.integral, display: true)
    }
}

/// A panel that never takes the keyboard from the toolbar's field.
private final class BarPanel: NSPanel {
    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// A hosting view a click reaches at once, though its window is not key:
/// the bar's panel never is, and a line clicked must run on the first
/// click rather than wake the window.
final class FirstMouseHostingView<Content: View>: NSHostingView<Content> {
    override func acceptsFirstMouse(for event: NSEvent?) -> Bool { true }
}

/// The toolbar's search field, which says when it takes the keyboard.
///
/// A click into the field opens the bar, as `/` does: the bar is what the
/// field is for, and typing into a field with no bar under it would be
/// words the controller never hears. `/` and ⌘K focus the field themselves
/// once the bar is up, which this reports too; the caller ignores it then.
public final class BarSearchField: NSSearchField {
    /// The field took the keyboard.
    public var onFocus: (() -> Void)?

    /// The rest of the best word, drawn in tertiary after the typed text
    /// while a short prefix is typed ("at|las", specs/010-focus-search
    /// screen 02). Tab takes it; the controller says what it is.
    public var ghost: String? {
        didSet {
            guard ghost != oldValue else { return }
            placeGhost()
        }
    }

    /// An operator's value is being typed: the text is SF Mono 14 (§2).
    public var operatorTyped = false {
        didSet {
            guard operatorTyped != oldValue else { return }
            if wordsFont == nil { wordsFont = font }
            font = operatorTyped ? .monospacedSystemFont(ofSize: 14, weight: .regular) : wordsFont
            placeGhost()
        }
    }

    private let ghostLabel: NSTextField = {
        let label = NSTextField(labelWithString: "")
        label.textColor = .tertiaryLabelColor
        label.isHidden = true
        label.setAccessibilityElement(false)
        return label
    }()
    private var wordsFont: NSFont?

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
        placeGhost()
    }

    /// The ghost, right after the typed text, in the field's font.
    private func placeGhost() {
        if ghostLabel.superview == nil { addSubview(ghostLabel) }
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
        // The field editor sets its text two points in (its line fragment
        // padding); the ghost follows the last glyph.
        ghostLabel.frame = NSRect(
            x: text.minX + 2 + width, y: (bounds.height - size.height) / 2,
            width: min(size.width, max(text.maxX - (text.minX + 2 + width), 0)), height: size.height)
        ghostLabel.isHidden = false
    }
}
