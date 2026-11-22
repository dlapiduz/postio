import AppKit
import SwiftUI

/// AppKit's `beginSheet` rather than SwiftUI's `.sheet`: a SwiftUI sheet hangs from one view's
/// window, and the digest's window is not SwiftUI's.
@MainActor
public final class FocusSheet {
    private var sheet: NSWindow?
    private weak var parent: NSWindow?
    private let onDismissed: @MainActor () -> Void

    public init(onDismissed: @escaping @MainActor () -> Void) {
        self.onDismissed = onDismissed
    }

    public var isShown: Bool { sheet != nil }

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

    public func close() {
        guard let sheet else { return }
        self.sheet = nil
        if let parent { parent.endSheet(sheet) } else { sheet.close() }
    }
}
