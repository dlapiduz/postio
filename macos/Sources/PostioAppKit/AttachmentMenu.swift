import AppKit
import PostioFFI

/// The menu `A` shows over the open message: what GTK's chooser offers, as a
/// native menu. The links in the body first, each as its words with where it
/// goes beneath them, so both are seen before it opens; then the attachments
/// by name and size, which open; then each attachment again under Save, and
/// Save all. The arrows, Return and Escape are AppKit's; nothing opens or is
/// written until an item is chosen.
@MainActor
public final class AttachmentMenu: NSObject {
    /// What a person chose.
    public enum Choice: Equatable, Sendable {
        /// Open the link to `target`.
        case link(String)
        /// Open attachment `id`.
        case open(Int64)
        /// Save attachment `id`, offering `name` in the panel.
        case save(id: Int64, name: String)
        /// Save every attachment to a folder.
        case saveAll
    }

    private let choose: (Choice) -> Void
    /// What each item of the last menu built stands for, by its tag.
    private var offered: [Choice] = []

    public init(choose: @escaping (Choice) -> Void) {
        self.choose = choose
    }

    /// The menu for `choices`, in GTK's order. The words are the FFI's; the
    /// ellipsis is the Mac's mark of an item that asks something more.
    public func menu(for choices: OpenChoicesFfi) -> NSMenu {
        let menu = NSMenu()
        menu.autoenablesItems = false
        offered = []
        for link in choices.links {
            let item = add(.link(link.target), title: link.words, to: menu)
            item.toolTip = link.target
            if #available(macOS 14.4, *) { item.subtitle = link.target }
        }
        if !choices.links.isEmpty, !choices.parts.isEmpty { menu.addItem(.separator()) }
        for part in choices.parts {
            add(.open(part.id), title: "\(part.name)  \(part.size)", to: menu)
        }
        if !choices.parts.isEmpty {
            menu.addItem(.separator())
            menu.addItem(.sectionHeader(title: choices.save))
            for part in choices.parts {
                add(.save(id: part.id, name: part.saveName), title: "\(part.name)\u{2026}", to: menu)
            }
            add(.saveAll, title: "\(choices.saveAll)\u{2026}", to: menu)
        }
        return menu
    }

    @discardableResult
    private func add(_ choice: Choice, title: String, to menu: NSMenu) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: #selector(chosen(_:)), keyEquivalent: "")
        item.target = self
        item.tag = offered.count
        offered.append(choice)
        menu.addItem(item)
        return item
    }

    /// Show the menu at the middle of `view`.
    public func present(_ choices: OpenChoicesFfi, in view: NSView) {
        let at = NSPoint(x: view.bounds.midX, y: view.bounds.midY)
        menu(for: choices).popUp(positioning: nil, at: at, in: view)
    }

    @objc private func chosen(_ item: NSMenuItem) {
        guard offered.indices.contains(item.tag) else { return }
        choose(offered[item.tag])
    }
}
