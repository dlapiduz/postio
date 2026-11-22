import AppKit
import PostioFFI
import PostioKit

/// One at a time (M4): another kind closes the first, the same kind replaces the content and keeps
/// the frame, so `j`/`k` never resize it. A child of the main window, so it moves and closes with
/// it; no minimise, since a minimised message is one the list no longer knows is open. Every close
/// is reported once through `onClosed`, which puts the keyboard back on the list. AppKit rather
/// than `openWindow`: a `WindowGroup` offers no parent, no placement against the main frame and no
/// close from an engine intent.
@MainActor
public final class SecondaryWindowController: NSObject, NSWindowDelegate {
    public static let heightInset: CGFloat = 80
    public static let minimumHeight: CGFloat = 320

    public private(set) var kind: SurfaceKindFfi?
    public private(set) var window: NSWindow?
    public private(set) weak var parent: NSWindow?

    private let present: @MainActor (NSWindow, NSWindow) -> Void
    private let onClosed: @MainActor (SurfaceKindFfi) -> Void
    private var parentObserver: NSObjectProtocol?

    public init(
        present: @escaping @MainActor (NSWindow, NSWindow) -> Void = SecondaryWindowController.presentAsChild,
        onClosed: @escaping @MainActor (SurfaceKindFfi) -> Void
    ) {
        self.present = present
        self.onClosed = onClosed
    }

    public static func presentAsChild(_ window: NSWindow, over parent: NSWindow) {
        parent.addChildWindow(window, ordered: .above)
        window.makeKeyAndOrderFront(nil)
    }

    public static func placed(width: CGFloat, height wanted: CGFloat? = nil, over parent: NSRect) -> NSRect {
        let tallest = max(minimumHeight, parent.height - heightInset)
        let height = wanted.map { min($0, tallest) } ?? tallest
        return NSRect(
            x: (parent.midX - width / 2).rounded(),
            y: (parent.midY - height / 2).rounded(),
            width: width,
            height: height)
    }

    public func show(
        _ kind: SurfaceKindFfi, content: NSView, width: CGFloat, height: CGFloat? = nil,
        title: String, over parent: NSWindow, configure: ((NSWindow) -> Void)? = nil
    ) {
        if let window, self.kind == kind {
            window.contentView = content
            window.title = title
            return
        }
        let frame = { Self.placed(width: width, height: height, over: parent.frame) }
        if let open = self.kind { close(open) }

        let window = NSWindow(
            contentRect: frame(),
            styleMask: [.titled, .closable, .resizable],
            backing: .buffered,
            defer: true)
        // ARC owns it: AppKit's default releases a window on close too, and the second release
        // crashes.
        window.isReleasedWhenClosed = false
        window.setFrame(frame(), display: false)
        window.title = title
        window.contentView = content
        window.delegate = self
        window.collectionBehavior.insert(.fullScreenAuxiliary)
        if kind == .message { KeyWindowTracker.tag(window, as: .message) }
        configure?(window)
        // Again: a toolbar added by `configure` grows the window by its height, and the frame is
        // the geometry's, not the content's.
        window.setFrame(frame(), display: false)

        self.window = window
        self.kind = kind
        self.parent = parent
        parentObserver = NotificationCenter.default.addObserver(
            forName: NSWindow.willCloseNotification, object: parent, queue: nil
        ) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, let kind = self.kind else { return }
                self.close(kind)
            }
        }
        present(window, parent)
    }

    public func fit(contentHeight: CGFloat) {
        guard let window, let parent else { return }
        let content = NSRect(x: 0, y: 0, width: window.frame.width, height: contentHeight)
        let wanted = window.frameRect(forContentRect: content).height
        let frame = Self.fitted(window.frame, height: wanted, over: parent.frame)
        guard frame != window.frame else { return }
        window.setFrame(frame, display: true)
    }

    public static func fitted(_ frame: NSRect, height wanted: CGFloat, over parent: NSRect) -> NSRect {
        let tallest = max(minimumHeight, parent.height - heightInset)
        let height = min(wanted, tallest)
        let lowest = (parent.midY - tallest / 2).rounded()
        return NSRect(
            x: frame.minX, y: max(frame.maxY - height, lowest), width: frame.width, height: height)
    }

    public func close(_ kind: SurfaceKindFfi) {
        guard self.kind == kind, let window else { return }
        window.close()
        if self.window === window { finish() }
    }

    public func windowWillClose(_ notification: Notification) {
        guard let closing = notification.object as? NSWindow, closing === window else { return }
        finish()
    }

    private func finish() {
        guard let kind, let window else { return }
        self.kind = nil
        self.window = nil
        window.delegate = nil
        if let parentObserver { NotificationCenter.default.removeObserver(parentObserver) }
        parentObserver = nil
        parent?.removeChildWindow(window)
        onClosed(kind)
    }
}
