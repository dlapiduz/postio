import Foundation
import PostioFFI

/// The menu bar, decided.
///
/// Separated from the `NSMenu` assembly because this is the half with
/// decisions in it — which commands appear, under which menu, in which order,
/// and what each one shows as its accelerator — and none of that needs AppKit
/// to be asserted. Building an `NSMenu` needs a running application; deciding
/// what should be in one does not, and a menu that is only checked by looking
/// at it is a menu nobody checks.
///
/// **Nothing here is a list of commands.** The rows come from
/// `PostioRegistry.commands` and the grouping from `postio_core::menu`, so a
/// command added on the Rust side appears here with no Swift change — which is
/// #657's whole point, and `PRODUCT.md` §8's rule from the other direction: a
/// command in the registry should not need an edit here to be discoverable.
public enum MenuPlan {
    /// One menu item.
    public struct Item: Equatable, Sendable {
        /// What choosing it does.
        public enum Role: Equatable, Sendable {
            /// Runs `command` through the path its key takes.
            case command
            /// Edit › Undo and Redo: the engine's stack in the main window,
            /// the field editor's in a text field (`UndoRouter`). Undo is the
            /// registry's `undo`.
            case undo, redo
            /// AppKit's text editing, down the responder chain.
            case cut, copy, paste, selectAll
            /// AppKit's application and window verbs.
            case about, hide, close, minimize, zoom
        }

        /// The registry id to invoke, or `nil` for an item that is AppKit's.
        public let command: String?
        /// What the item says.
        public let title: String
        /// The binding in force, as macOS draws it — `⌘K`, `⇧⌘A`, `G`.
        ///
        /// `nil` when the command has no binding, and deliberately also for a
        /// *sequence*: `g g` cannot be drawn as an accelerator and drawing
        /// only its first chord would be a lie about which key runs it. The
        /// cheat sheet is where sequences are described in full, which is why
        /// it lives in the Help menu.
        public let shortcut: String?
        /// What choosing it does.
        public let role: Role

        public init(command: String?, title: String, shortcut: String?, role: Role = .command) {
            self.command = command
            self.title = title
            self.shortcut = shortcut
            self.role = role
        }
    }

    /// One top-level menu, with what belongs under it.
    public struct Menu: Equatable, Sendable {
        public let title: String
        public let items: [Item]
        /// Which menu this is, so the builder can tell the application menu
        /// from the rest without matching on its title.
        public let section: MenuSectionFfi?

        public init(title: String, items: [Item], section: MenuSectionFfi?) {
            self.title = title
            self.items = items
            self.section = section
        }
    }

    /// The whole menu bar, in order, empty menus dropped.
    ///
    /// `binding` is asked per command rather than read off `defaultBinding`,
    /// because a menu drawing the default for a command somebody rebound is
    /// confidently wrong — worse for a menu item than showing no key at all.
    public static func build(
        commands: [CommandSpecFfi] = PostioRegistry.commands,
        menus: [MenuFfi] = PostioFFI.menus(),
        bindings: (String) -> [String]
    ) -> [Menu] {
        menus.compactMap { menu in
            let items =
                commands
                .filter { $0.menu == menu.section }
                .map { spec in
                    Item(
                        command: spec.id,
                        title: spec.title,
                        shortcut: accelerator(among: bindings(spec.id))
                    )
                }
            // A menu with nothing under it draws as an empty pane, which reads
            // as a broken application rather than as a section that happens to
            // be empty on this build.
            return items.isEmpty ? nil : Menu(title: menu.title, items: items, section: menu.section)
        }
    }

    /// Which of a command's bindings a menu draws.
    ///
    /// **The chord, when there is one.** Both keyboard layers are always on —
    /// `e` replies and so does `⌘R` — but a menu is where a Mac user looks
    /// for the second, and the shortcut column is shaped for it. The
    /// mnemonic is not lost: the cheat sheet lists every binding, which is
    /// what it is for.
    ///
    /// Falls back to the primary, so a command with only a mnemonic still
    /// shows it rather than showing nothing.
    public static func accelerator(among bindings: [String]) -> String? {
        // Anything longer than one character and not a sequence: `cmd+r`,
        // and also `Down`, which is a chord in every sense a menu cares
        // about — the canvas draws `↓` beside "next thread" and `j` is the
        // mnemonic under it. A bare letter is the mnemonic layer, and it is
        // the fallback rather than the choice.
        let chord = bindings.first { $0.count > 1 && !$0.contains(" ") }
        return (chord ?? bindings.first).flatMap(accelerator(from:))
    }

    /// A binding string as macOS draws it, or `nil` if it cannot be drawn.
    ///
    /// The core hands over a resolved binding — `cmd+k`, `shift+cmd+a`, `g g`
    /// — already expanded for this platform (`mod` is `cmd` here). Rendering
    /// it into glyphs is the frontend's job and only the frontend's: ADR 0019
    /// Q4 gives each frontend "a small renderer from that trigger into its
    /// platform's accelerator format", `<Ctrl>N` for GTK and `⌘N` here.
    ///
    /// A sequence answers `nil`. `⌘` glyphs cannot express "press g, then g",
    /// and an item showing `G` for a command that `G` does not run is worse
    /// than one showing nothing.
    public static func accelerator(from binding: String) -> String? {
        guard !binding.contains(" ") else { return nil }
        var parts = binding.split(separator: "+").map(String.init)
        guard let key = parts.popLast(), !key.isEmpty else { return nil }

        // Apple's order, which is not the order the binding string uses:
        // ⌃⌥⇧⌘, always, whatever sequence somebody typed into `[keys]`.
        var glyphs = ""
        let held = Set(parts.map { $0.lowercased() })
        if held.contains("ctrl") || held.contains("control") { glyphs += "⌃" }
        if held.contains("alt") || held.contains("option") { glyphs += "⌥" }
        // Shift is written two ways and they mean the same key. The resolver
        // *folds* it into the character for a key that types one — `shift+a`
        // **is** `A`, because that is what a keyboard delivers — so eleven
        // registry defaults are bare capitals with no `shift` in the string.
        // Uppercasing them for display threw the fold away and drew `A` for
        // both `archive` and `archive_thread`: two commands, two keys, one
        // accelerator. Seen in the Message menu, invisible to every test that
        // existed before this one.
        if held.contains("shift") || isFoldedShift(key) { glyphs += "⇧" }
        if held.contains("cmd") || held.contains("command") || held.contains("super")
            || held.contains("meta")
        {
            glyphs += "⌘"
        }
        return glyphs + keyGlyph(key)
    }

    /// Whether `key` is a character that already carries a folded Shift.
    ///
    /// A single uppercase letter, and only that. `A` is `shift+a`; `Return`
    /// is a named key that merely starts with a capital, and `F5` is not
    /// shifted either.
    private static func isFoldedShift(_ key: String) -> Bool {
        guard key.count == 1, let only = key.first else { return false }
        return only.isUppercase && only.isLetter
    }

    /// The key half, as a menu draws it.
    ///
    /// The named keys are the resolver's own spellings, which are GDK's; the
    /// glyphs are the ones a Mac user reads without thinking. Anything else is
    /// a single character and is shown uppercased, the way every menu on the
    /// system shows it — an uppercase letter in a menu is not a claim about
    /// Shift, which has its own glyph.
    ///
    /// The punctuation names are `postio_ui::keymap`'s `PUNCTUATION_NAMES`,
    /// which is why they are here at all: `mod+comma` is Settings' default,
    /// and a menu that printed the *name* showed `⌘COMMA` — not a key anybody
    /// can find on a keyboard. This is the rendering table ADR 0019 Q4 says
    /// each frontend owns, and `noAcceleratorLeaksAKeyName` is what notices
    /// when the core learns a name this does not know.
    private static func keyGlyph(_ key: String) -> String {
        // The keypad's keys are GDK's `KP_…` names; a Mac draws the
        // character the key types (`zoom_in`'s `KP_Add` is `+`), and its
        // Enter as ⌅.
        let lowered = key.lowercased()
        if lowered.hasPrefix("kp_") {
            let name = String(lowered.dropFirst(3))
            switch name {
            case "add": return "+"
            case "subtract": return "-"
            case "multiply": return "*"
            case "divide": return "/"
            case "decimal": return "."
            case "equal": return "="
            case "enter": return "⌅"
            default:
                if name.count == 1, name.first?.isNumber == true { return name }
            }
        }
        switch lowered {
        case "comma": return ","
        case "period": return "."
        case "slash": return "/"
        case "backslash": return "\\"
        case "question": return "?"
        case "semicolon": return ";"
        case "colon": return ":"
        case "plus": return "+"
        case "minus": return "-"
        case "equal": return "="
        case "asterisk": return "*"
        case "underscore": return "_"
        case "less": return "<"
        case "greater": return ">"
        case "bracketleft": return "["
        case "bracketright": return "]"
        case "grave": return "`"
        case "apostrophe": return "'"
        case "quotedbl": return "\""
        case "return", "enter": return "↩"
        case "escape", "esc": return "⎋"
        case "tab": return "⇥"
        case "space": return "␣"
        case "backspace": return "⌫"
        case "delete": return "⌦"
        case "up": return "↑"
        case "down": return "↓"
        case "left": return "←"
        case "right": return "→"
        case "home": return "↖"
        case "end": return "↘"
        case "page_up", "pageup": return "⇞"
        case "page_down", "pagedown": return "⇟"
        default: return key.uppercased()
        }
    }
}



extension MenuPlan {
    /// Focus's whole menu bar on the Mac (specs/009-focus-macos T105): the
    /// registry's menus for what Focus offers here (`build`), with the
    /// standard items every Mac application has put around them.
    ///
    /// - **App:** About, the registry's three (Settings ⌘, first), Hide,
    ///   then the registry's Quit ⌘Q -- once; AppKit's own Quit went, it was
    ///   the same verb drawn twice.
    /// - **File:** the registry's, Compose titled "New Message" as every Mac
    ///   mail application titles it (⌘N, the same command as `c`), then
    ///   Close ⌘W, which is AppKit's: a window closes on that chord through a
    ///   menu item and nowhere else. File, not Window, because that is where
    ///   a Mac user looks for it (the HIG's File menu).
    /// - **Edit:** Undo -- the registry's `undo`, run by `UndoRouter` so a
    ///   text field keeps its own -- Redo, Cut, Copy, Paste, Select All, then
    ///   the registry's other Edit commands.
    /// - **Window:** Minimize and Zoom, AppKit's; it keeps its window list
    ///   itself. Before Help, where macOS puts it.
    ///
    /// Every registry item's shortcut is `accelerator(among: bindings(id))`,
    /// the key in force, so a `[keys]` change shows on the next build.
    public static func bar(
        commands: [CommandSpecFfi] = PostioRegistry.commands,
        menus: [MenuFfi] = PostioFFI.menus(),
        bindings: (String) -> [String]
    ) -> [Menu] {
        let planned = build(commands: commands, menus: menus, bindings: bindings)
        var bar: [Menu] = []
        // The application menu is there even if the registry put nothing in
        // it: About, Hide and Quit are the platform's.
        let appMenu = planned.first { $0.section == .app }
        let app = appMenu?.items ?? []
        let quit = app.filter { $0.command == Standard.quit }
        bar.append(
            Menu(
                title: appMenu?.title ?? "Postio",
                items: [Item(command: nil, title: "About Postio", shortcut: nil, role: .about)]
                    + app.filter { $0.command != Standard.quit }
                    + [Item(command: nil, title: "Hide Postio", shortcut: "⌘H", role: .hide)]
                    + quit,
                section: .app))
        for menu in planned where menu.section != .app {
            if menu.section == .help { bar.append(window) }
            switch menu.section {
            case .file:
                let items = menu.items.map { item in
                    item.command == Standard.compose
                        ? Item(command: item.command, title: "New Message", shortcut: item.shortcut)
                        : item
                }
                bar.append(
                    Menu(
                        title: menu.title,
                        items: items + [Item(command: nil, title: "Close", shortcut: "⌘W", role: .close)],
                        section: .file))
            case .edit:
                let undo = menu.items.first { $0.command == Standard.undo }
                let editing: [Item] = [
                    Item(
                        command: Standard.undo, title: "Undo",
                        shortcut: undo?.shortcut ?? "⌘Z", role: .undo),
                    Item(command: nil, title: "Redo", shortcut: "⇧⌘Z", role: .redo),
                    Item(command: nil, title: "Cut", shortcut: "⌘X", role: .cut),
                    Item(command: nil, title: "Copy", shortcut: "⌘C", role: .copy),
                    Item(command: nil, title: "Paste", shortcut: "⌘V", role: .paste),
                    Item(command: nil, title: "Select All", shortcut: "⌘A", role: .selectAll),
                ]
                bar.append(
                    Menu(
                        title: menu.title,
                        items: editing + menu.items.filter { $0.command != Standard.undo },
                        section: .edit))
            default:
                bar.append(menu)
            }
        }
        if !bar.contains(where: { $0.title == window.title }) { bar.append(window) }
        return bar
    }

    /// The registry ids the standard items stand for.
    enum Standard {
        static let undo = "undo"
        static let quit = "quit"
        static let compose = "compose"
    }

    /// The Window menu: AppKit's verbs; the window list is AppKit's own.
    static let window = Menu(
        title: "Window",
        items: [
            Item(command: nil, title: "Minimize", shortcut: "⌘M", role: .minimize),
            Item(command: nil, title: "Zoom", shortcut: nil, role: .zoom),
        ],
        section: nil)
}

/// Focus's menu bar as it stands, rebuilt when `[keys]` changes
/// (specs/009-focus-macos T105): `KeymapChanged` is said after the session
/// has the new keymap, so `bindingsFor` answers the new keys by then.
@MainActor
public final class MenuBarPlan {
    public private(set) var menus: [MenuPlan.Menu]
    private let bindings: (String) -> [String]

    public init(bindings: @escaping (String) -> [String]) {
        self.bindings = bindings
        menus = MenuPlan.bar(bindings: bindings)
    }

    /// Plan the bar again from the bindings in force.
    public func rebuild() {
        menus = MenuPlan.bar(bindings: bindings)
    }

    /// Rebuild on `KeymapChanged`; `true` when it did.
    @discardableResult
    public func apply(_ event: UiEvent) -> Bool {
        guard case .keymapChanged = event else { return false }
        rebuild()
        return true
    }
}
