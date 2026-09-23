import Foundation
import PostioFFI

/// The window's toolbar, decided (canvas screen 25).
///
/// Separated from the toolbar itself for the reason `MenuPlan` is: which
/// verbs get a button, in which order, with which symbol and which words, is
/// a decision — and a toolbar that is only ever checked by looking at it is a
/// toolbar nobody checks.
///
/// **Every item is a registry command.** A toolbar button that did its own
/// thing would be a fourth way to archive that undo does not know about, and
/// the canvas' rule is the same one the row's hover actions follow: the mouse
/// and the keyboard run one code path.
///
/// Icons only, no labels — and therefore a tooltip and a menu-bar equivalent
/// on every one of them, which is the price of an unlabelled control on this
/// platform. The tooltip carries the accelerator, so the toolbar teaches the
/// keyboard rather than competing with it.
public enum ToolbarPlan {
    public struct Item: Equatable, Sendable {
        /// The registry id to invoke.
        public let command: String
        /// The SF Symbol drawn on the button.
        public let symbol: String
        /// What VoiceOver says, and what the tooltip starts with.
        public let title: String
    }

    /// The buttons, in the canvas' order: archive, flag, reply, new message.
    ///
    /// Four, and stopping there is the decision. A toolbar is not a list of
    /// everything the application can do — that is the palette, which is one
    /// keystroke away and searchable. These four are what the canvas draws
    /// and what a hand reaches for without reading.
    public static let items: [Item] = [
        Item(command: "archive", symbol: "archivebox", title: "Archive"),
        Item(command: "flag", symbol: "flag", title: "Flag"),
        Item(command: "reply", symbol: "arrowshape.turn.up.left", title: "Reply"),
        Item(command: "compose", symbol: "square.and.pencil", title: "New message"),
    ]

    /// The tooltip for `item`: what it does, and the key that does it.
    ///
    /// The accelerator comes from the binding in force rather than from the
    /// default, so a rebound key is what the tooltip says — the same rule
    /// `MenuPlan` follows, and for the same reason: a control that names the
    /// wrong key is worse than one that names none.
    public static func tooltip(for item: Item, bindings: (String) -> [String]) -> String {
        guard let chord = MenuPlan.accelerator(among: bindings(item.command)) else {
            return item.title
        }
        return "\(item.title) (\(chord))"
    }
}
