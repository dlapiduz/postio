import AppKit
import SwiftUI
import PostioKit

/// A programmatic `FocusState` change on a SwiftUI `TextField` hosted in `NSToolbar` does not cross
/// into AppKit, and SwiftUI then re-seats the keyboard on the window's first focusable view
/// (#1260), so the push is made in AppKit's terms. "Nearest" climbs from the searcher in the
/// field's background, so it means this item's field, not whichever the window holds.
public enum ViewTreeFocus {
    public static func firstTextField(under root: NSView) -> NSTextField? {
        if let field = root as? NSTextField, field.isEditable {
            return field
        }
        for subview in root.subviews {
            if let field = firstTextField(under: subview) {
                return field
            }
        }
        return nil
    }

    /// The climb is bounded: past the toolbar item that hosts both views "nearest" means nothing,
    /// and an unbounded walk reaches the window.
    public static func nearestTextField(from leaf: NSView, climbing limit: Int = 8) -> NSTextField? {
        var node: NSView? = leaf
        var steps = 0
        while let current = node, steps <= limit {
            if let field = firstTextField(under: current) {
                return field
            }
            node = current.superview
            steps += 1
        }
        return nil
    }
}

/// Watches a count, not a flag: the field is always on the toolbar, so search can be asked for
/// while already showing, and an unchanged Bool cannot carry the second ask.
public struct ToolbarFieldFocus: NSViewRepresentable {
    public let asks: Int
    /// Escape is swallowed by the key monitor before the field sees it, so leaving search is the
    /// engine's decision and arrives as this turning false.
    public let wanted: Bool

    public init(asks: Int, wanted: Bool) {
        self.asks = asks
        self.wanted = wanted
    }

    public final class Coordinator {
        var granted = 0
    }

    public func makeCoordinator() -> Coordinator { Coordinator() }

    public func makeNSView(context: Context) -> NSView {
        // Creation is not an ask: a grant here would steal the keyboard at launch.
        context.coordinator.granted = asks
        return NSView(frame: .zero)
    }

    public func updateNSView(_ view: NSView, context: Context) {
        if context.coordinator.granted != asks {
            context.coordinator.granted = asks
            // After the update: AppKit refuses first-responder changes made while SwiftUI is
            // mid-layout.
            DispatchQueue.main.async {
                guard let field = ViewTreeFocus.nearestTextField(from: view) else { return }
                field.window?.makeFirstResponder(field)
            }
        } else if !wanted {
            DispatchQueue.main.async {
                // Resign only when it is this field's editor holding the keyboard (a focused
                // NSTextField hands typing to the window's field editor, whose delegate is the
                // field).
                guard let field = ViewTreeFocus.nearestTextField(from: view),
                      let window = field.window,
                      let editor = window.firstResponder as? NSText,
                      editor.delegate === field
                else { return }
                window.makeFirstResponder(nil)
            }
        }
    }
}
