import AppKit
import PostioKit

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
@MainActor
enum MenuBar {
    /// Build the bar and install it, routing every choice through `run`.
    static func install(
        binding: @escaping (String) -> String?,
        available: @escaping (String) -> Bool,
        run: @escaping (String) -> Void
    ) {
        let target = CommandTarget(run: run, available: available)
        Self.target = target

        let bar = NSMenu()
        // The application menu, which is AppKit's and not the registry's:
        // About, Hide, Quit. A Mac without it is not a Mac, and none of it is
        // a Postio command.
        let appItem = NSMenuItem()
        appItem.submenu = applicationMenu()
        bar.addItem(appItem)

        for menu in MenuPlan.build(binding: binding) {
            let item = NSMenuItem()
            let submenu = NSMenu(title: menu.title)
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
        windows.addItem(
            withTitle: "Minimize", action: #selector(NSWindow.performMiniaturize(_:)), keyEquivalent: "m")
        windows.addItem(withTitle: "Zoom", action: #selector(NSWindow.performZoom(_:)), keyEquivalent: "")
        windowItem.submenu = windows
        bar.addItem(windowItem)

        NSApp.mainMenu = bar
        NSApp.windowsMenu = windows
    }

    /// Held for as long as the menu is: `NSMenuItem` keeps an unowned target,
    /// and a deallocated one makes every item stop working with no error.
    private static var target: CommandTarget?

    private static func applicationMenu() -> NSMenu {
        let menu = NSMenu()
        menu.addItem(
            withTitle: "About Postio",
            action: #selector(NSApplication.orderFrontStandardAboutPanel(_:)),
            keyEquivalent: "")
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
