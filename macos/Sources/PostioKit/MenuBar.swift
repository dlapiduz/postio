import AppKit

/// Assembling the menu bar from `MenuPlan`.
///
/// The decisions are all in `MenuPlan`, in PostioKit, where they are asserted
/// without AppKit. This is the part that needs a running application: turning
/// a decided list into `NSMenu`s and hanging them off `NSApp`.
///
/// # No key equivalents, on purpose
///
/// Every item here shows its accelerator and **none of them has one**
/// (`keyEquivalent` stays empty). Dispatch is `KeyMonitor`'s, and two things
/// claiming one keystroke is a race whose winner depends on AppKit's event
/// routing rather than on anything Postio decided (ADR 0019 Q4).
///
/// So the accelerator is drawn rather than set: an attributed title with the
/// glyphs right-aligned in secondary colour, which is what the standard
/// shortcut column looks like. It also does something a key equivalent cannot
/// — it can be *absent* for a sequence like `g g`, which has no accelerator
/// spelling at all, instead of showing a first chord that does not run the
/// command.
///
/// # It has to be installed *and stay* installed
///
/// SwiftUI builds the main menu from its own scenes, and it does not do it by
/// replacing `NSApp.mainMenu`: it **mutates the menu that is already there**,
/// dropping submenus it does not know about and adding its own. Measured on
/// the running application — `mainMenu === ours` was true the whole time
/// while the bar on screen read `Postio File Edit View Window Help`, with Go,
/// Message and Format gone (#1262).
///
/// Two consequences, and both were wrong in the first two attempts at this.
/// Identity is not the test — the shape is. And no notification is late
/// enough to be the trigger: did-become-active, did-become-key and
/// did-finish-launching all fire before the rebuild that matters, so a
/// reassert there restores a bar SwiftUI then edits again. What works is
/// watching the menu itself for items arriving and leaving, and rebuilding
/// when the shape stops being ours.
@MainActor
public enum MenuBar {
    /// Build the bar and install it, routing every choice through `run`.
    public static func install(
        bindings: @escaping (String) -> [String],
        available: @escaping (String) -> Bool,
        run: @escaping (String) -> Void
    ) {
        recipe = Recipe(bindings: bindings, available: available, run: run)
        mount()
    }

    /// What `install` was asked for, kept so the bar can be built again.
    ///
    /// It has to be built again rather than merely re-pointed-at: SwiftUI
    /// edits the menu in place, so what is on screen after it has had its
    /// turn is our object with its submenus removed.
    private struct Recipe {
        let bindings: (String) -> [String]
        let available: (String) -> Bool
        let run: (String) -> Void
    }

    private static var recipe: Recipe?

    /// Build the bar from the recipe and hang it off the application.
    private static func mount() {
        guard let recipe else { return }
        let target = CommandTarget(run: recipe.run, available: recipe.available)
        Self.target = target
        let bindings = recipe.bindings

        let bar = NSMenu()
        // The application menu, which is AppKit's and not the registry's:
        // About, Hide, Quit are AppKit's and not the registry's. The registry
        // does own three of this menu's items -- Settings, the config file and
        // Add account -- and they are merged in rather than drawn as a second
        // "Postio" menu beside it (#1207).
        let planned = MenuPlan.build(bindings: bindings)
        let appItem = NSMenuItem()
        appItem.submenu = applicationMenu(
            items: planned.first { $0.section == .app }?.items ?? [],
            target: target
        )
        bar.addItem(appItem)

        for menu in planned where menu.section != .app {
            let item = NSMenuItem()
            let submenu = NSMenu(title: menu.title)
            // Edit gets AppKit's own editing items first. **This is what makes
            // paste work at all**: ⌘V reaches a text field only because a menu
            // item carries it as a key equivalent and sends `paste:` down the
            // responder chain. Replacing SwiftUI's bar (#1262) took its Edit
            // menu with it, and with it ⌘V, ⌘C, ⌘X, ⌘A and ⌘Z everywhere in
            // the application (#1298) -- found by a password that could not be pasted
            // into the add-account sheet.
            //
            // No conflict with the registry: Postio's modified defaults are
            // `ctrl+…`, which on this platform is ⌃ and not ⌘, so every one of
            // these chords is unclaimed. And `KeyMonitor` is a local event
            // monitor, which runs before menu key equivalents are considered
            // -- so a future ⌘-binding of Postio's own would still win, and
            // these stay the fallback rather than becoming a race.
            if menu.section == .edit {
                appendStandardEditing(to: submenu)
                if !menu.items.isEmpty { submenu.addItem(.separator()) }
            }
            for planned in menu.items {
                submenu.addItem(menuItem(for: planned, target: target))
            }
            item.submenu = submenu
            bar.addItem(item)
        }

        // Window, also AppKit's: minimise, zoom, and the window list it keeps
        // itself. Naming it is what makes `NSApp.windowsMenu` work.
        let windowItem = NSMenuItem()
        let windows = NSMenu(title: "Window")
        // **Close comes first, and ⌘W only works because it is here.** A
        // window closes on that chord through a menu item and nowhere else,
        // exactly as ⌘V pastes through one — so replacing SwiftUI's bar
        // (#1262) took both away together, and neither is recoverable by any
        // amount of correct work elsewhere.
        //
        // Convention puts Close under File; this bar's File menu is the
        // registry's, and mixing an AppKit window verb into it would make the
        // registry's own list a half-truth. Window is where the other two
        // window verbs already are, and is where it can sit beside them
        // without either menu lying about what it owns.
        windows.addItem(
            withTitle: "Close", action: #selector(NSWindow.performClose(_:)), keyEquivalent: "w")
        windows.addItem(.separator())
        windows.addItem(
            withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windows.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        windowItem.submenu = windows
        bar.addItem(windowItem)

        installed = bar
        expected = bar.items.compactMap { $0.submenu?.title }
        // `NSApplication.shared`, not `NSApp`: the bar is installed at launch
        // now, and at launch `Engine` is built inside `App.init()` — before
        // SwiftUI has created the application object, so `NSApp` is nil and an
        // implicitly-unwrapped nil is a crash on the program's first line.
        let app = NSApplication.shared
        app.mainMenu = bar
        app.windowsMenu = windows
        watch()
    }

    /// The bar this application built, or `nil` before one was installed.
    private(set) static var installed: NSMenu?

    /// The submenu titles the bar is supposed to have.
    private static var expected: [String] = []

    /// Whether the menu bar on screen is Postio's.
    ///
    /// The question #1262 turned on, and the reason it is a property rather
    /// than an assumption: everything in `MenuPlan` — the accelerators, the
    /// context validation, the application-menu placement — was correct and
    /// invisible, because nothing ever asked this.
    public static var isMounted: Bool {
        guard installed != nil, !expected.isEmpty else { return false }
        return shape(of: NSApplication.shared.mainMenu) == expected
    }

    /// The submenu titles of `menu`, which is what "the bar on screen" means
    /// to anyone looking at it.
    private static func shape(of menu: NSMenu?) -> [String] {
        menu?.items.compactMap { $0.submenu?.title } ?? []
    }

    /// Put the bar back if something replaced it.
    ///
    /// Idempotent and cheap: a pointer comparison, done at the two moments
    /// SwiftUI rebuilds.
    public static func reassert() {
        guard recipe != nil, !isMounted, !rebuilding else { return }
        rebuilding = true
        defer { rebuilding = false }
        mount()
        remounts += 1
    }

    /// Guards the rebuild against itself: building a menu adds items to it,
    /// and the observer that noticed items arriving is still registered.
    private static var rebuilding = false

    /// How many times the bar had to be put back.
    ///
    /// Not diagnostics for their own sake: "it is on screen now" and "it is
    /// on screen because something keeps restoring it" are different states,
    /// and #1262 was a day of the first looking like the second.
    public private(set) static var remounts = 0

    /// Watch for the rebuilds. Registered once, however often `install` runs.
    ///
    /// **Key-value observation on `mainMenu` itself**, rather than on the
    /// notifications around it. SwiftUI rebuilds the menu whenever its scene
    /// graph updates, and every notification worth listening to — did become
    /// active, did become key, did finish launching — fires *before* the
    /// rebuild that matters: reasserting there restored a bar that SwiftUI
    /// then replaced again, and the application ran with AppKit's stock menus
    /// while every observer said it had done its job. Watching the property
    /// is the only thing that cannot be early.
    ///
    /// The pointer check in `reassert` is what makes this terminate: our own
    /// write fires the observer, which sees the bar is already ours and stops.
    private static func watch() {
        guard observers.isEmpty else { return }
        for name in [NSMenu.didAddItemNotification, NSMenu.didRemoveItemNotification] {
            observers.append(
                NotificationCenter.default.addObserver(
                    forName: name, object: nil, queue: .main
                ) { _ in
                    // Any menu, deliberately: the notification's object is
                    // not `Sendable`, and the question it would answer —
                    // "was that the bar?" — is answered anyway by the shape
                    // check inside `reassert`, which is a string comparison
                    // over six titles. Asynchronously, so SwiftUI finishes
                    // whatever edit it is part way through before the shape
                    // is judged.
                    DispatchQueue.main.async { reassert() }
                }
            )
        }
    }

    private static var observers: [NSObjectProtocol] = []

    /// Held for as long as the menu is: `NSMenuItem` keeps an unowned target,
    /// and a deallocated one makes every item stop working with no error.
    private static var target: CommandTarget?

    /// AppKit's application menu, with the registry's three items in it.
    ///
    /// Where macOS puts them, and the only place `⌘,` is discoverable here.
    /// Until this existed the menu was About/Hide/Quit and Settings fell back
    /// to Edit, so the shortcut was announced nowhere once the mail loaded.
    /// AppKit's editing items, with the key equivalents that make them work.
    ///
    /// The selectors are sent down the responder chain, so each item enables
    /// itself only when something focused can perform it -- which is why
    /// `Paste` is grey with no text field in front and live with one, without
    /// this file knowing anything about which surfaces take text.
    ///
    /// `undo:` and `redo:` have no formal declaration to take a `#selector`
    /// of; they are `NSResponder`'s by convention, and the string is the
    /// spelling every application on this platform uses.
    static func appendStandardEditing(to menu: NSMenu) {
        menu.addItem(withTitle: "Undo", action: Selector(("undo:")), keyEquivalent: "z")
        let redo = menu.addItem(
            withTitle: "Redo", action: Selector(("redo:")), keyEquivalent: "z")
        redo.keyEquivalentModifierMask = [.command, .shift]
        menu.addItem(.separator())
        menu.addItem(withTitle: "Cut", action: #selector(NSText.cut(_:)), keyEquivalent: "x")
        menu.addItem(withTitle: "Copy", action: #selector(NSText.copy(_:)), keyEquivalent: "c")
        menu.addItem(withTitle: "Paste", action: #selector(NSText.paste(_:)), keyEquivalent: "v")
        menu.addItem(
            withTitle: "Select All", action: #selector(NSText.selectAll(_:)), keyEquivalent: "a")
    }

    private static func applicationMenu(items: [MenuPlan.Item], target: CommandTarget) -> NSMenu {
        let menu = NSMenu()
        menu.addItem(
            withTitle: "About Postio",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: "")
        if !items.isEmpty {
            menu.addItem(.separator())
            for planned in items {
                menu.addItem(menuItem(for: planned, target: target))
            }
        }
        menu.addItem(.separator())
        menu.addItem(
            withTitle: "Hide Postio", action: #selector(NSApplication.hide(_:)), keyEquivalent: "h")
        menu.addItem(.separator())
        menu.addItem(
            withTitle: "Quit Postio", action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q")
        return menu
    }

    private static func menuItem(for planned: MenuPlan.Item, target: CommandTarget) -> NSMenuItem {
        let item = NSMenuItem(
            title: planned.title,
            action: #selector(CommandTarget.run(_:)),
            // Empty, always. See the note above: the monitor dispatches.
            keyEquivalent: ""
        )
        item.target = target
        item.representedObject = planned.command
        if let shortcut = planned.shortcut {
            item.attributedTitle = attributed(planned.title, shortcut: shortcut)
            // An attributed title *becomes* the accessibility name, tab
            // character and glyph included -- VoiceOver read "Command palette
            // tab ⌘K". The drawn accelerator is a hint for people who can see
            // it; the name is what gets spoken, and it is the command.
            // `PRODUCT.md` §20, found by reading the menu out of the running
            // application rather than by looking at it.
            item.setAccessibilityTitle(planned.title)
        }
        return item
    }

    /// A title with its accelerator drawn where the shortcut column is.
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

/// The object every menu item points at.
///
/// One target for the whole bar, carrying the registry id on the item itself,
/// rather than a selector per command — which would be a list of commands
/// written in Swift, and is exactly what #657 exists to prevent.
///
/// It is also what validates: `NSMenu` asks its items' target before opening,
/// so this is where an item the build cannot run is greyed. The answer comes
/// from the boundary, so the menu and the palette ask the same question and a
/// command added in Rust is filtered with no Swift change (#1158).
@MainActor
private final class CommandTarget: NSObject, NSMenuItemValidation {
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

    /// AppKit asks this for every item whose target we are, each time a menu
    /// is about to open.
    func validateMenuItem(_ item: NSMenuItem) -> Bool {
        guard let id = item.representedObject as? String else { return true }
        return available(id)
    }
}
