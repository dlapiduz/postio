import AppKit
import PostioFFI
import PostioKit
import SwiftUI

@MainActor
public final class ComposeWindowChrome: NSObject, NSToolbarDelegate {
    static let title = NSToolbarItem.Identifier("postio.compose.title")
    static let trailing = NSToolbarItem.Identifier("postio.compose.trailing")

    private let model: ComposeModel
    private let sendCap: String?
    private let send: () -> Void
    private let sendAt: (Int64) -> Void
    private let toolbar = NSToolbar(identifier: "PostioFocusCompose")

    public init(
        model: ComposeModel, sendCap: String?,
        send: @escaping () -> Void, sendAt: @escaping (Int64) -> Void
    ) {
        self.model = model
        self.sendCap = sendCap
        self.send = send
        self.sendAt = sendAt
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
        window.titlebarSeparatorStyle = .line
    }

    public func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [.flexibleSpace, Self.title, .flexibleSpace, Self.trailing]
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
            let view = NSHostingView(rootView: ComposeTitle(model: model))
            view.sizingOptions = [.intrinsicContentSize]
            item.view = view
            item.label = "Message"
        case Self.trailing:
            let view = NSHostingView(
                rootView: ComposeTrailing(sendCap: sendCap, send: send, sendAt: sendAt))
            view.sizingOptions = [.intrinsicContentSize]
            item.view = view
            item.label = "Send"
        default:
            return nil
        }
        item.isBordered = false
        return item
    }
}

struct ComposeTitle: View {
    let model: ComposeModel

    var body: some View {
        VStack(spacing: 1) {
            Text(model.heading)
                .font(.system(size: 13.5, weight: .bold))
            if let saved = model.savedWords {
                Text(saved)
                    .font(.system(size: 12))
                    .foregroundStyle(.secondary)
            }
        }
        .accessibilityElement(children: .combine)
    }
}

struct ComposeTrailing: View {
    let sendCap: String?
    let send: () -> Void
    let sendAt: (Int64) -> Void

    var body: some View {
        HStack(spacing: 10) {
            Menu {
                ForEach(schedulePresets(), id: \.when) { preset in
                    Button(preset.label) { sendAt(preset.when) }
                }
            } label: {
                Text("Send later").font(.system(size: 13.5, weight: .semibold))
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .accessibilityLabel("Send later")
            FocusDefaultButton("Send", cap: sendCap, action: send)
        }
        .padding(.trailing, 4)
    }
}
