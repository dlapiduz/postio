import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// A child of the main window, so it moves with it; it never becomes key, so the results keep the
/// keyboard.
@MainActor
public final class QuickLookPanel {
    private let model: QuickLookModel
    private let present: (NSPanel, NSWindow) -> Void

    public private(set) var panel: NSPanel?

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
            panel?.invalidateShadow()
        case .close:
            guard let panel else { return }
            panel.parent?.removeChildWindow(panel)
            panel.orderOut(nil)
            self.panel = nil
        }
    }

    private func frame(over main: NSWindow) -> NSRect {
        let size = NSSize(width: QuickLookMetrics.width, height: QuickLookMetrics.height)
        let window = main.frame
        let x = window.minX + max(0, (window.width - size.width) / 2)
        let y = max(window.minY, window.maxY - QuickLookMetrics.top - size.height)
        return NSRect(origin: NSPoint(x: x, y: y), size: size)
    }

    private final class Panel: NSPanel {
        override var canBecomeKey: Bool { false }
        override var canBecomeMain: Bool { false }
    }
}
