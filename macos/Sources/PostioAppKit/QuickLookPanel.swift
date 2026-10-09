import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// Quick Look's presenter (specs/010-focus-search T091; design §3.7,
/// screen 10): a floating panel over the results, 780 by 470 with a radius
/// of 14 and the window server's shadow, and nothing behind it dimmed.
///
/// It is a child of the main window, centred across it and 160 under its
/// top, so it moves with it; it never becomes key, so the results keep the
/// keyboard and Space, j/k, ]/[, ↩, `a` and Esc stay the controller's. It
/// opens and closes when `QuickLookModel` says the controller did; a new
/// view while it is up is new content in the same panel (`QuickLookBody`
/// observes the model), never a second one.
@MainActor
public final class QuickLookPanel {
    private let model: QuickLookModel
    private let present: (NSPanel, NSWindow) -> Void

    /// The panel, while Quick Look is open.
    public private(set) var panel: NSPanel?

    /// `present` orders the panel in over the main window; a test replaces
    /// it so nothing is put on screen.
    public init(
        model: QuickLookModel,
        present: @escaping (NSPanel, NSWindow) -> Void = { panel, main in
            main.addChildWindow(panel, ordered: .above)
            panel.orderFront(nil)
        }
    ) {
        self.model = model
        self.present = present
    }

    /// Show what `change` says, over `main`.
    public func apply(_ change: QuickLookModel.Change, over main: NSWindow?) {
        switch change {
        case .open:
            guard panel == nil, let main else { return }
            let panel = Panel(
                contentRect: frame(over: main),
                styleMask: [.borderless, .nonactivatingPanel],
                backing: .buffered, defer: true)
            panel.isReleasedWhenClosed = false
            panel.isOpaque = false
            panel.backgroundColor = .clear
            panel.hasShadow = true
            panel.hidesOnDeactivate = false
            panel.animationBehavior = .none
            let content = NSHostingView(rootView: QuickLookBody(model: model))
            content.frame = NSRect(origin: .zero, size: panel.frame.size)
            content.autoresizingMask = [.width, .height]
            panel.contentView = content
            panel.setFrame(frame(over: main), display: false)
            panel.setAccessibilityLabel(model.view?.title ?? "Quick Look")
            self.panel = panel
            present(panel, main)
        case .redraw:
            // The body reads the model: the content follows, the panel stays.
            panel?.invalidateShadow()
        case .close:
            guard let panel else { return }
            panel.parent?.removeChildWindow(panel)
            panel.orderOut(nil)
            self.panel = nil
        }
    }

    /// 780 by 470, centred across `main`, 160 under its top; kept inside it
    /// when the window is smaller than that.
    private func frame(over main: NSWindow) -> NSRect {
        let size = NSSize(width: QuickLookMetrics.width, height: QuickLookMetrics.height)
        let window = main.frame
        let x = window.minX + max(0, (window.width - size.width) / 2)
        let y = max(window.minY, window.maxY - QuickLookMetrics.top - size.height)
        return NSRect(origin: NSPoint(x: x, y: y), size: size)
    }

    /// A panel that never takes the keyboard from the results.
    private final class Panel: NSPanel {
        override var canBecomeKey: Bool { false }
        override var canBecomeMain: Bool { false }
    }
}
