import AppKit
import PostioKit
import SwiftUI

@MainActor
public final class MessageWindowChrome: NSObject, NSToolbarDelegate {
    static let title = NSToolbarItem.Identifier("postio.message.title")
    static let stepper = NSToolbarItem.Identifier("postio.message.stepper")

    private let model: MessageWindowModel
    private let binding: (String) -> String?
    private let run: (String) -> Void
    private let toolbar = NSToolbar(identifier: "PostioFocusMessage")

    public init(
        model: MessageWindowModel, binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void
    ) {
        self.model = model
        self.binding = binding
        self.run = run
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        toolbar.centeredItemIdentifiers = [Self.title]
    }

    public func install(on window: NSWindow) {
        window.titleVisibility = .hidden
        window.toolbarStyle = .unified
        window.toolbar = toolbar
        window.titlebarSeparatorStyle = .none
        window.backgroundColor = .windowBackgroundColor
    }

    public func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.flexibleSpace, Self.title, .flexibleSpace, Self.stepper]
    }

    public func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    public func toolbar(
        _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        let item = NSToolbarItem(itemIdentifier: identifier)
        switch identifier {
        case Self.title:
            let title = NSHostingView(rootView: MessageWindowTitle(model: model, binding: binding))
            title.sizingOptions = [.intrinsicContentSize]
            item.view = title
            item.label = "Message"
        case Self.stepper:
            item.view = NSHostingView(
                rootView: MessageWindowStepper(model: model, binding: binding, run: run))
            item.label = "Previous and next"
        default:
            return nil
        }
        item.isBordered = false
        return item
    }
}
