import AppKit
import PostioFFI

/// The menu `A` shows over the open message; nothing opens or is written until an item is chosen.
@MainActor
public final class AttachmentMenu: NSObject {
    public enum Choice: Equatable, Sendable {
        case link(String)
        case open(Int64)
        case save(id: Int64, name: String)
        case saveAll
    }

    private let choose: (Choice) -> Void
    private var offered: [Choice] = []

    public init(choose: @escaping (Choice) -> Void) {
        self.choose = choose
    }

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

    public func present(_ choices: OpenChoicesFfi, in view: NSView) {
        let at = NSPoint(x: view.bounds.midX, y: view.bounds.midY)
        menu(for: choices).popUp(positioning: nil, at: at, in: view)
    }

    @objc private func chosen(_ item: NSMenuItem) {
        guard offered.indices.contains(item.tag) else { return }
        choose(offered[item.tag])
    }
}
