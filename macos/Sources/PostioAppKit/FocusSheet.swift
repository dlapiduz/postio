import AppKit
import SwiftUI

/// A sheet the controller opens and closes -- the digest-this-sender
/// sheet (specs/009-focus-macos T115) -- on whichever window has the
/// keyboard: the main window from the list, the digest's from a digest
/// (`d` edits its rule there). AppKit's `beginSheet` rather than SwiftUI's
/// `.sheet`, because a SwiftUI sheet hangs from one view's window, and the
/// digest's window is not SwiftUI's.
///
/// `onDismissed` hears a close the toolkit made, which is none today --
/// the sheet has no close button, and Escape is the controller's Back --
/// but a parent window closing under it would be one.
@MainActor
public final class FocusSheet {
    private var sheet: NSWindow?
    private weak var parent: NSWindow?
    private let onDismissed: @MainActor () -> Void

    public init(onDismissed: @escaping @MainActor () -> Void) {
        self.onDismissed = onDismissed
    }

    /// Whether the sheet is up.
    public var isShown: Bool { sheet != nil }

    /// Show `content` as a sheet on `window`.
    public func show<Content: View>(_ content: Content, on window: NSWindow) {
        if sheet != nil { return }
        let hosting = NSHostingController(rootView: content)
        hosting.sizingOptions = [.preferredContentSize]
        let sheet = NSWindow(contentViewController: hosting)
        sheet.styleMask = [.titled, .docModalWindow]
        sheet.isReleasedWhenClosed = false
        self.sheet = sheet
        parent = window
        window.beginSheet(sheet) { [weak self] _ in
            MainActor.assumeIsolated {
                guard let self, self.sheet === sheet else { return }
                self.sheet = nil
                self.onDismissed()
            }
        }
    }

    /// Take the sheet down: the controller closed it.
    public func close() {
        guard let sheet else { return }
        self.sheet = nil
        if let parent { parent.endSheet(sheet) } else { sheet.close() }
    }
}
