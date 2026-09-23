import AppKit
import SwiftUI

/// Finding the text field a toolbar item hosts, so the keyboard can be moved
/// there from AppKit when SwiftUI will not (#1260).
///
/// `/` resolved, `showingSearch` flipped, `@FocusState` was set — and the
/// keyboard never moved. A SwiftUI `TextField` in the window's toolbar is
/// hosted inside `NSToolbar`, and a programmatic `FocusState` change does not
/// cross that boundary: focus driven *from* AppKit (a click into the field)
/// is reported back into SwiftUI correctly, but focus pushed the other way is
/// dropped, and SwiftUI then re-seats the keyboard on the window's first
/// focusable view — which is how pressing `/` landed the focus on the
/// *sidebar*. So the push is made in AppKit's own terms: find the
/// `NSTextField` and make it first responder.
///
/// "Nearest" is what keeps this honest. The searcher sits in the field's own
/// background, so climbing one ancestor at a time and taking the first
/// editable field means "this toolbar item's field", not "whichever field the
/// window happens to hold".
public enum ViewTreeFocus {
    /// The first editable text field under `root`, depth-first.
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

    /// The editable text field nearest `leaf`: climb one ancestor at a time,
    /// searching each ancestor's whole subtree before climbing again.
    ///
    /// The climb is bounded because past the toolbar item that hosts both
    /// views, "nearest" stops meaning anything: an unbounded walk reaches the
    /// window and hands back whichever field it happens to contain.
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

/// Pushes the keyboard into the toolbar's text field when a command asks.
///
/// Sits in the field's `.background`, which is what gives
/// [`ViewTreeFocus.nearestTextField`] its anchor. Watches a **count**, not a
/// flag: the field is always on the toolbar, so `search` can be asked for
/// while search is already showing, and a Bool that does not change cannot
/// carry the second ask — the same token shape as every wish granted on this
/// branch.
public struct ToolbarFieldFocus: NSViewRepresentable {
    /// How many times focus has been asked for.
    public let asks: Int
    /// Whether the field should hold the keyboard at all — the resign half.
    /// Escape is swallowed by the key monitor before the field ever sees it
    /// (an `.onKeyPress` on a claimed key is dead code), so leaving search is
    /// the engine's decision and arrives here as this turning false.
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
        // The view being created is not somebody asking for focus: a grant
        // on creation would steal the keyboard at launch.
        context.coordinator.granted = asks
        return NSView(frame: .zero)
    }

    public func updateNSView(_ view: NSView, context: Context) {
        if context.coordinator.granted != asks {
            context.coordinator.granted = asks
            // After the update, not during it: AppKit refuses first-responder
            // changes made while SwiftUI is mid-layout.
            DispatchQueue.main.async {
                guard let field = ViewTreeFocus.nearestTextField(from: view) else { return }
                field.window?.makeFirstResponder(field)
            }
        } else if !wanted {
            DispatchQueue.main.async {
                // Resign only when it is *our* field's editor holding the
                // keyboard — a focused NSTextField hands typing to the
                // window's field editor, whose delegate is the field.
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
