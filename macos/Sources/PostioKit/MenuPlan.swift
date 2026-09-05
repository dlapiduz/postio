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
        /// The registry id to invoke.
        public let command: String
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
    }

    /// One top-level menu, with what belongs under it.
    public struct Menu: Equatable, Sendable {
        public let title: String
        public let items: [Item]
    }

    /// The whole menu bar, in order, empty menus dropped.
    ///
    /// `binding` is asked per command rather than read off `defaultBinding`,
    /// because a menu drawing the default for a command somebody rebound is
    /// confidently wrong — worse for a menu item than showing no key at all.
    public static func build(
        commands: [CommandSpecFfi] = PostioRegistry.commands,
        menus: [MenuFfi] = PostioFFI.menus(),
        binding: (String) -> String?
    ) -> [Menu] {
        menus.compactMap { menu in
            let items =
                commands
                .filter { $0.menu == menu.section }
                .map { spec in
                    Item(
                        command: spec.id,
                        title: spec.title,
                        shortcut: binding(spec.id).flatMap(accelerator(from:))
                    )
                }
            // A menu with nothing under it draws as an empty pane, which reads
            // as a broken application rather than as a section that happens to
            // be empty on this build.
            return items.isEmpty ? nil : Menu(title: menu.title, items: items)
        }
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
        switch key.lowercased() {
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
