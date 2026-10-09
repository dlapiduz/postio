import AppKit
import PostioFFI
import PostioKit

/// The window over Focus's list: the message, and later the digest, the
/// composer and capture (specs/009-focus-macos T066,
/// contracts/mac-surfaces.md "Secondary windows").
///
/// - **One at a time** (M4). Opening another kind closes the first and
///   reports it closed; the same kind again replaces the content and keeps
///   the frame, so `j` and `k` never resize the message window and a size
///   the person chose survives the next message.
/// - **Placed** centred over the main window, as wide as the engine's
///   geometry says (M1, `FocusReaderDocumentFfi.windowWidth`) and as tall
///   as the main window less 80. The list behind is not dimmed: this is a
///   window of its own, not a sheet.
/// - **A child of the main window**, so it moves with it and closes with
///   it. It has traffic lights and no minimise: a message minimised to the
///   Dock is a message the list no longer knows is open.
/// - **Every close is reported once**, through `onClosed`: ⌘W and the close
///   button (`windowWillClose`), the controller's `FocusCloseSurface`
///   (`close(_:)`), another kind replacing it, and the main window closing.
///   The caller tells the engine (`focusSurfaceClosed`), which is what puts
///   the keyboard back on the list.
///
/// # Why AppKit, not SwiftUI's `openWindow`
///
/// The app makes its other windows as SwiftUI scenes (`PostioApp`), and
/// that is right for Settings and compose, which stand alone. This window
/// is not: it must be a *child* of the main window (moves with it, closes
/// with it), be placed against the main window's frame, keep its frame
/// across content changes, and be closed by the engine's intent rather
/// than by a view. A `WindowGroup` scene offers none of those -- no parent,
/// no placement but its own default, and a close that only a view's
/// `dismissWindow` can ask for -- and each would be a workaround reaching
/// for the `NSWindow` underneath anyway. So the window is AppKit's and its
/// content is SwiftUI, hosted.
@MainActor
public final class SecondaryWindowController: NSObject, NSWindowDelegate {
    /// How much shorter than the main window it is (M1).
    public static let heightInset: CGFloat = 80
    /// The least height it is given, whatever the main window's.
    public static let minimumHeight: CGFloat = 320

    /// Which surface is open, or `nil`.
    public private(set) var kind: SurfaceKindFfi?
    /// Its window, while it is open.
    public private(set) var window: NSWindow?
    /// The main window it belongs to.
    public private(set) weak var parent: NSWindow?

    private let present: @MainActor (NSWindow, NSWindow) -> Void
    private let onClosed: @MainActor (SurfaceKindFfi) -> Void
    private var parentObserver: NSObjectProtocol?

    /// `present` orders the window in over its parent; the default makes it
    /// a child and gives it the keyboard. `onClosed` is told each close.
    public init(
        present: @escaping @MainActor (NSWindow, NSWindow) -> Void = SecondaryWindowController.presentAsChild,
        onClosed: @escaping @MainActor (SurfaceKindFfi) -> Void
    ) {
        self.present = present
        self.onClosed = onClosed
    }

    /// The default `present`: a child of `parent`, in front, with the
    /// keyboard.
    public static func presentAsChild(_ window: NSWindow, over parent: NSWindow) {
        parent.addChildWindow(window, ordered: .above)
        window.makeKeyAndOrderFront(nil)
    }

    /// Where a window `width` wide goes over `parent`: centred both ways,
    /// the parent's height less `heightInset`, or `height` when the surface
    /// asks for less (capture, a form rather than a page to read).
    public static func placed(width: CGFloat, height wanted: CGFloat? = nil, over parent: NSRect) -> NSRect {
        let tallest = max(minimumHeight, parent.height - heightInset)
        let height = wanted.map { min($0, tallest) } ?? tallest
        return NSRect(
            x: (parent.midX - width / 2).rounded(),
            y: (parent.midY - height / 2).rounded(),
            width: width,
            height: height)
    }

    /// Show `content` as `kind` over `parent`: in the open window when it
    /// is the same kind (its frame kept), else in a new one `width` wide.
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
        // ARC owns it. AppKit's default releases a window on close as well,
        // and the second release is a crash the next time it is touched.
        window.isReleasedWhenClosed = false
        window.setFrame(frame(), display: false)
        window.title = title
        window.contentView = content
        window.delegate = self
        window.collectionBehavior.insert(.fullScreenAuxiliary)
        if kind == .message { KeyWindowTracker.tag(window, as: .message) }
        configure?(window)
        // Again: a toolbar `configure` added keeps the content's size and
        // grows the window by its own height (the message window's title
        // area is one), and the frame is the geometry's, not the content's.
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

    /// Close `kind` if it is the one open; nothing otherwise. Reported
    /// through `onClosed`, once.
    public func close(_ kind: SurfaceKindFfi) {
        guard self.kind == kind, let window else { return }
        // `windowWillClose` does the rest, as for ⌘W.
        window.close()
        if self.window === window { finish() }
    }

    // MARK: NSWindowDelegate

    public func windowWillClose(_ notification: Notification) {
        guard let closing = notification.object as? NSWindow, closing === window else { return }
        finish()
    }

    /// Forget the window and say which kind closed.
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
