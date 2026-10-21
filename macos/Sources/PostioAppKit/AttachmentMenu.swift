import AppKit
import PostioFFI

/// The menu `o` shows over the open message: its attachments, each as its
/// name and size, the way the chips say them. A native menu, so the arrows,
/// Return and Escape are AppKit's; nothing opens until an item is chosen.
@MainActor
public final class AttachmentMenu: NSObject {
    private let choose: (Int64) -> Void

    public init(choose: @escaping (Int64) -> Void) {
        self.choose = choose
    }

    /// One item per part, in the order the chips draw them.
    public func menu(for parts: [FocusAttachmentFfi]) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        for part in parts {
            let item = NSMenuItem(
                title: "\(part.name)  \(part.size)", action: #selector(chosen(_:)), keyEquivalent: "")
            item.target = self
            item.representedObject = NSNumber(value: part.id)
            menu.addItem(item)
        }
        return menu
    }

    /// Show the menu at the middle of `view`.
    public func present(_ parts: [FocusAttachmentFfi], in view: NSView) {
        let at = NSPoint(x: view.bounds.midX, y: view.bounds.midY)
        menu(for: parts).popUp(positioning: nil, at: at, in: view)
    }

    @objc private func chosen(_ item: NSMenuItem) {
        guard let id = (item.representedObject as? NSNumber)?.int64Value else { return }
        choose(id)
    }
}
