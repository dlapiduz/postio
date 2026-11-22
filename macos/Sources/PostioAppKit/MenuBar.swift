import AppKit
import os
import PostioKit

/// Every item shows its accelerator and none has a key equivalent: dispatch is `KeyMonitor`'s, and
/// two
/// things claiming one keystroke is a race (ADR 0019 Q4). The accelerator is drawn in an attributed
/// title, so it can be absent for `g g`.
/// SwiftUI mutates the existing `mainMenu` in place, dropping submenus it does not know, so
/// identity is no
/// test; the shape is. No notification fires late enough, so the menu is watched for items arriving
/// and leaving (#1262).
@MainActor
public enum MenuBar {
    public static func install(
        menus: @escaping () -> [MenuPlan.Menu],
        available: @escaping (String) -> Bool,
        run: @escaping (String) -> Void,
        undo: UndoRouter? = nil
    ) {
        recipe = Recipe(menus: menus, available: available, run: run, undo: undo)
        mount()
    }

    public static func install(
        bindings: @escaping (String) -> [String],
        available: @escaping (String) -> Bool,
        run: @escaping (String) -> Void,
        undo: UndoRouter? = nil
    ) {
        install(menus: { MenuPlan.bar(bindings: bindings) }, available: available, run: run, undo: undo)
    }

    /// Rebuilt rather than re-pointed: SwiftUI edits the menu in place, leaving our object with
    /// submenus removed.
    private struct Recipe {
        let menus: () -> [MenuPlan.Menu]
        let available: (String) -> Bool
        let run: (String) -> Void
        let undo: UndoRouter?
    }

    private static var recipe: Recipe?

    /// AppKit's own items (Undo, Redo, Cut, Copy, Paste, Select All, Hide, Close, Minimize) carry
    /// real key
    /// equivalents: on this platform those keys reach a text field or window only through a menu
    /// item (#1262, #1298).
    /// No race with Postio's keys: `KeyMonitor` runs before menu key equivalents, and in a text
    /// field `KeyDisposition.belongsToText` hands ⌘A, ⌘C, ⌘V, ⌘X, ⌘Z to the field.
    private static func mount() {
        guard let recipe else { return }
        let target = CommandTarget(run: recipe.run, available: recipe.available)
        Self.target = target

        let bar = NSMenu()
        var windows: NSMenu?
        for menu in recipe.menus() {
            let item = NSMenuItem()
            let submenu = Self.menu(for: menu, target: target, undo: recipe.undo)
            item.submenu = submenu
            bar.addItem(item)
            if menu.section == nil, menu.title == "Window" { windows = submenu }
        }

        installed = bar
        expected = bar.items.compactMap { $0.submenu?.title }
        // `NSApplication.shared`, not `NSApp`: `Engine` is built inside `App.init()`, before
        // SwiftUI creates the application object, so `NSApp` is nil.
        let app = NSApplication.shared
        app.mainMenu = bar
        if let windows { app.windowsMenu = windows }
        watch()
    }

    static func menu(for planned: MenuPlan.Menu, target: CommandTarget?, undo router: UndoRouter?) -> NSMenu {
        let menu = NSMenu(title: planned.title)
        var previous: MenuPlan.Item.Role?
        for item in planned.items {
            if let previous,
               (previous == .command) != (item.role == .command) || previous == .redo
            {
                menu.addItem(.separator())
            }
            menu.addItem(menuItem(for: item, target: target, undo: router))
            previous = item.role
        }
        return menu
    }

    static func menuItem(
        for planned: MenuPlan.Item, target: CommandTarget?, undo router: UndoRouter?
    ) -> NSMenuItem {
        func standard(_ action: Selector, _ key: String, _ modifiers: NSEvent.ModifierFlags = .command)
            -> NSMenuItem
        {
            let item = NSMenuItem(title: planned.title, action: action, keyEquivalent: key)
            item.keyEquivalentModifierMask = modifiers
            return item
        }
        switch planned.role {
        case .command:
            return commandItem(for: planned, target: target)
        case .undo:
            let item = standard(UndoRouter.undoAction, "z")
            item.target = router
            return item
        case .redo:
            let item = standard(UndoRouter.redoAction, "z", [.command, .shift])
            item.target = router
            return item
        case .cut: return standard(#selector(NSText.cut(_:)), "x")
        case .copy: return standard(#selector(NSText.copy(_:)), "c")
        case .paste: return standard(#selector(NSText.paste(_:)), "v")
        case .selectAll: return standard(#selector(NSText.selectAll(_:)), "a")
        case .about: return standard(#selector(NSApplication.orderFrontStandardAboutPanel(_:)), "")
        case .hide: return standard(#selector(NSApplication.hide(_:)), "h")
        case .close: return standard(#selector(NSWindow.performClose(_:)), "w")
        case .minimize: return standard(#selector(NSWindow.performMiniaturize(_:)), "m")
        case .zoom: return standard(#selector(NSWindow.performZoom(_:)), "")
        }
    }

    private(set) static var installed: NSMenu?

    private static var expected: [String] = []

    public static var isMounted: Bool {
        guard installed != nil, !expected.isEmpty else { return false }
        return shape(of: NSApplication.shared.mainMenu) == expected
    }

    private static func shape(of menu: NSMenu?) -> [String] {
        menu?.items.compactMap { $0.submenu?.title } ?? []
    }

    public static func reassert() {
        guard recipe != nil, !isMounted, !rebuilding else { return }
        rebuilding = true
        defer { rebuilding = false }
        // Top-level titles only: the Window menu lists open windows, and a compose window's title
        // is a message's subject.
        let found = shape(of: NSApplication.shared.mainMenu)
        mount()
        remounts += 1
        log.notice(
            "menu bar put back (\(remounts, privacy: .public)): found \(found.joined(separator: " "), privacy: .public)"
        )
    }

    private static let log = Logger(subsystem: "dev.postio.Postio", category: "menu")

    /// Building a menu adds items to it, and the observer that noticed items arriving is still
    /// registered.
    private static var rebuilding = false

    public private(set) static var remounts = 0

    /// Key-value observation on the menu rather than the notifications around it: every one fires
    /// before
    /// the rebuild that matters, so reasserting there restored a bar SwiftUI then replaced again.
    /// The pointer check in `reassert` makes this terminate.
    private static func watch() {
        guard observers.isEmpty else { return }
        for name in [NSMenu.didAddItemNotification, NSMenu.didRemoveItemNotification] {
            observers.append(
                NotificationCenter.default.addObserver(
                    forName: name, object: nil, queue: .main
                ) { _ in
                    // Any menu: the notification's object is not `Sendable`, and the shape check in
                    // `reassert`
                    // answers "was that the bar?". Async, so SwiftUI finishes its edit first.
                    DispatchQueue.main.async { reassert() }
                }
            )
        }
    }

    private static var observers: [NSObjectProtocol] = []

    /// `NSMenuItem` keeps an unowned target; a deallocated one makes every item stop working with
    /// no error.
    private static var target: CommandTarget?

    private static func commandItem(for planned: MenuPlan.Item, target: CommandTarget?) -> NSMenuItem {
        let item = NSMenuItem(
            title: planned.title,
            action: #selector(CommandTarget.run(_:)),
            keyEquivalent: ""
        )
        item.target = target
        item.representedObject = planned.command
        if let shortcut = planned.shortcut {
            item.attributedTitle = attributed(planned.title, shortcut: shortcut)
            // An attributed title becomes the accessibility name, tab and glyph included (VoiceOver
            // read
            // "Command palette tab ⌘K"), so the plain title is set.
            item.setAccessibilityTitle(planned.title)
        }
        return item
    }

    private static func attributed(_ title: String, shortcut: String) -> NSAttributedString {
        let paragraph = NSMutableParagraphStyle()
        paragraph.tabStops = [NSTextTab(textAlignment: .right, location: 260)]
        let text = NSMutableAttributedString(
            string: "\(title)\t",
            attributes: [.paragraphStyle: paragraph]
        )
        text.append(
            NSAttributedString(
                string: shortcut,
                attributes: [
                    .paragraphStyle: paragraph,
                    .foregroundColor: NSColor.secondaryLabelColor,
                ]
            )
        )
        return text
    }
}

/// One target for the whole bar, with the registry id on the item, rather than a selector per
/// command.
/// It also validates: the boundary answers, so a command added in Rust is filtered with no Swift
/// change (#1158).
@MainActor
final class CommandTarget: NSObject, NSMenuItemValidation {
    private let runner: (String) -> Void
    private let available: (String) -> Bool

    init(run: @escaping (String) -> Void, available: @escaping (String) -> Bool) {
        runner = run
        self.available = available
    }

    @objc func run(_ sender: NSMenuItem) {
        guard let id = sender.representedObject as? String else { return }
        runner(id)
    }

    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        guard let id = item.representedObject as? String else { return true }
        return available(id)
    }
}
