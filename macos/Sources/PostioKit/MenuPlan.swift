import Foundation
import PostioFFI

/// Nothing here is a list of commands: rows come from `PostioRegistry.commands` and grouping from
/// `postio_core::menu`, so a command added in Rust appears with no Swift change (#657).
public enum MenuPlan {
    public struct Item: Equatable, Sendable {
        public enum Role: Equatable, Sendable {
            case command
            case undo, redo
            case cut, copy, paste, selectAll
            case about, hide, close, minimize, zoom
        }

        public let command: String?
        public let title: String
        /// `nil` for a sequence too: `g g` cannot be drawn as an accelerator, and drawing only its
        /// first chord would lie about which key runs it.
        public let shortcut: String?
        public let role: Role

        public init(command: String?, title: String, shortcut: String?, role: Role = .command) {
            self.command = command
            self.title = title
            self.shortcut = shortcut
            self.role = role
        }
    }

    public struct Menu: Equatable, Sendable {
        public let title: String
        public let items: [Item]
        public let section: MenuSectionFfi?

        public init(title: String, items: [Item], section: MenuSectionFfi?) {
            self.title = title
            self.items = items
            self.section = section
        }
    }

    /// `binding` is asked per command, not read off `defaultBinding`: a menu drawing the default
    /// for a rebound command is confidently wrong.
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
            return items.isEmpty ? nil : Menu(title: menu.title, items: items, section: menu.section)
        }
    }

    /// The chord when there is one: both keyboard layers are always on, but a menu is where a Mac
    /// user looks for the chord.
    /// Falls back to the primary so a mnemonic-only command still shows it.
    public static func accelerator(among bindings: [String]) -> String? {
        let chord = bindings.first { $0.count > 1 && !$0.contains(" ") }
        return (chord ?? bindings.first).flatMap(accelerator(from:))
    }

    /// A sequence answers `nil`: `⌘` glyphs cannot express "press g, then g", and an item showing
    /// `G` for a command `G` does not run is worse than none.
    /// Rendering is the frontend's job (ADR 0019 Q4).
    public static func accelerator(from binding: String) -> String? {
        guard !binding.contains(" ") else { return nil }
        var parts = binding.split(separator: "+").map(String.init)
        guard let key = parts.popLast(), !key.isEmpty else { return nil }

        var glyphs = ""
        let held = Set(parts.map { $0.lowercased() })
        if held.contains("ctrl") || held.contains("control") { glyphs += "⌃" }
        if held.contains("alt") || held.contains("option") { glyphs += "⌥" }
        // Shift is written two ways: the resolver folds it into the character (`shift+a` is `A`),
        // so bare capitals carry no `shift`.
        // Uppercasing them for display drew `A` for both `archive` and `archive_thread`.
        if held.contains("shift") || isFoldedShift(key) { glyphs += "⇧" }
        if held.contains("cmd") || held.contains("command") || held.contains("super")
            || held.contains("meta")
        {
            glyphs += "⌘"
        }
        return glyphs + keyGlyph(key)
    }

    private static func isFoldedShift(_ key: String) -> Bool {
        guard key.count == 1, let only = key.first else { return false }
        return only.isUppercase && only.isLetter
    }

    /// The named keys are the resolver's spellings (GDK's). The punctuation names are
    /// `postio_ui::keymap`'s `PUNCTUATION_NAMES`:
    /// without them `mod+comma` showed `⌘COMMA`. `noAcceleratorLeaksAKeyName` notices when the core
    /// learns a name this does not know.
    private static func keyGlyph(_ key: String) -> String {
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
        case "numbersign": return "#"
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
    /// The standard items every Mac application has are put around the registry's menus. Compose is
    /// titled "New Message"; Close ⌘W is AppKit's
    /// (a window closes on that chord only through a menu item); Undo is run by `UndoRouter` so a
    /// text field keeps its own; the registry's Quit replaces AppKit's.
    public static func bar(
        commands: [CommandSpecFfi] = PostioRegistry.commands,
        menus: [MenuFfi] = PostioFFI.menus(),
        bindings: (String) -> [String]
    ) -> [Menu] {
        let planned = build(commands: commands, menus: menus, bindings: bindings)
        var bar: [Menu] = []
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

    enum Standard {
        static let undo = "undo"
        static let quit = "quit"
        static let compose = "compose"
    }

    static let window = Menu(
        title: "Window",
        items: [
            Item(command: nil, title: "Minimize", shortcut: "⌘M", role: .minimize),
            Item(command: nil, title: "Zoom", shortcut: nil, role: .zoom),
        ],
        section: nil)
}

@MainActor
public final class MenuBarPlan {
    public private(set) var menus: [MenuPlan.Menu]
    private let bindings: (String) -> [String]

    public init(bindings: @escaping (String) -> [String]) {
        self.bindings = bindings
        menus = MenuPlan.bar(bindings: bindings)
    }

    public func rebuild() {
        menus = MenuPlan.bar(bindings: bindings)
    }

    @discardableResult
    public func apply(_ event: UiEvent) -> Bool {
        guard case .keymapChanged = event else { return false }
        rebuild()
        return true
    }
}
