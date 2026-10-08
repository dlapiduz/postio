import AppKit
import PostioKit
import SwiftUI

/// The digest window's title area (specs/009-focus-macos T114, screens 22
/// and 23): a unified toolbar, as the message window's
/// (`MessageWindowChrome`), so the title bar is 52pt with the traffic
/// lights centred in it. "‹ Summary" on the left while an email is open,
/// the title and its line in the centre, and on the right "Archive all"
/// or, on an email, the `k j` cap and the previous/next pair. The views are
/// SwiftUI's (`DigestBack`, `DigestTitle`, `DigestTrailing`), reading the
/// model, so a new view redraws them.
@MainActor
public final class DigestWindowChrome: NSObject, NSToolbarDelegate {
    static let back = NSToolbarItem.Identifier("postio.digest.back")
    static let title = NSToolbarItem.Identifier("postio.digest.title")
    static let trailing = NSToolbarItem.Identifier("postio.digest.trailing")

    private let model: DigestModel
    private let toolbar = NSToolbar(identifier: "PostioFocusDigest")

    public init(model: DigestModel) {
        self.model = model
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        toolbar.centeredItemIdentifiers = [Self.title]
    }

    /// Put the title area on `window`.
    public func install(on window: NSWindow) {
        window.titleVisibility = .hidden
        window.toolbarStyle = .unified
        window.toolbar = toolbar
        window.titlebarSeparatorStyle = .none
        window.backgroundColor = .windowBackgroundColor
    }

    public func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [Self.back, .flexibleSpace, Self.title, .flexibleSpace, Self.trailing]
    }

    public func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    public func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        let view: NSView
        switch identifier {
        case Self.back:
            view = NSHostingView(rootView: DigestBack(model: model))
            item.label = "Back"
        case Self.title:
            view = NSHostingView(rootView: DigestTitle(model: model))
            item.label = "Digest"
        case Self.trailing:
            view = NSHostingView(rootView: DigestTrailing(model: model))
            item.label = "Archive all"
        default:
            return nil
        }
        (view as? NSHostingView<DigestBack>)?.sizingOptions = [.intrinsicContentSize]
        (view as? NSHostingView<DigestTitle>)?.sizingOptions = [.intrinsicContentSize]
        (view as? NSHostingView<DigestTrailing>)?.sizingOptions = [.intrinsicContentSize]
        item.view = view
        item.isBordered = false
        return item
    }
}
