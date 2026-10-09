import AppKit
import PostioFFI
import PostioKit
import SwiftUI

/// The composer's title area (specs/009-focus-macos T079, screens 05 and
/// 06): a unified toolbar, as the message window's and the digest's, so
/// the title bar is 52pt with the traffic lights centred in it. What this
/// is ("New message", "Reply to all") and when it was saved ("Draft saved
/// locally 16:12") in the centre; Send later ▾ and Send ⌘↩ on the right,
/// Send a default button filled in the label colour.
@MainActor
public final class ComposeWindowChrome: NSObject, NSToolbarDelegate {
    static let title = NSToolbarItem.Identifier("postio.compose.title")
    static let trailing = NSToolbarItem.Identifier("postio.compose.trailing")

    private let model: ComposeModel
    private let sendCap: String?
    private let send: () -> Void
    private let sendAt: (Int64) -> Void
    private let toolbar = NSToolbar(identifier: "PostioFocusCompose")

    /// `send` is ⌘↩'s, `sendAt` a Send later time's (epoch milliseconds).
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

    /// Put the title area on `window`.
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

/// "New message", and under it "Draft saved locally 16:12" once saved.
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

/// Send later ▾ and Send ⌘↩.
struct ComposeTrailing: View {
    let sendCap: String?
    let send: () -> Void
    let sendAt: (Int64) -> Void

    var body: some View {
        HStack(spacing: 10) {
            Menu {
                // The boundary's four times, worked out as the menu opens.
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
