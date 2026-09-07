import Foundation
import PostioFFI

/// The bar under an open message, decided.
///
/// Separated from the view for `ToolbarPlan`'s reason: which verbs a reader
/// is offered, in what order, and which one is prominent is a decision — and
/// a bar that is only ever checked by looking at it is a bar nobody checks.
/// #1259 is what that costs: three of the four verbs had buttons, `archive`
/// had none, and nothing failed.
///
/// **Which four and in what order is not decided here.** That list is
/// `postio_ui::reader::header::ReaderAction`, arriving through
/// `Session.readerActions()`, so GTK and this frontend cannot end up offering
/// different bars. What this adds is the two things that are genuinely the
/// platform's: how a chord is spelled (`⇧⌘A`, not `mod+shift+a`), and
/// whether the verb can run where the pointer is.
public enum ReaderActionPlan {
    public struct Item: Equatable, Sendable {
        /// The registry id to invoke — never a local implementation, or the
        /// button is a second way to archive that undo does not know about.
        public let command: String
        /// What the button is labelled.
        public let title: String
        /// Whether it gets the prominent treatment. Exactly one does.
        public let prominent: Bool
        /// Whether it can run on what the pane is showing.
        public let enabled: Bool
        /// The key that does the same thing, spelled the way macOS spells it,
        /// or `nil` when nothing is bound to it.
        public let chord: String?
    }

    /// The bar, in the order the engine gave.
    ///
    /// A verb that cannot run here is **disabled, not dropped**: a bar that
    /// changed shape as the cursor moved would make "where did Archive go" a
    /// question, and a greyed button is a better answer than an absent one.
    ///
    /// The chord comes from the binding in force rather than from the
    /// registry's default, so a rebinding reaches the button — `MenuPlan`'s
    /// rule, and the same reason a control naming the wrong key is worse than
    /// one naming none.
    public static func items(
        from offered: [ReaderActionFfi],
        available: (String) -> Bool,
        bindings: (String) -> [String]
    ) -> [Item] {
        offered.map { action in
            Item(
                command: action.command,
                title: action.title,
                prominent: action.primary,
                enabled: available(action.command),
                chord: MenuPlan.accelerator(among: bindings(action.command))
            )
        }
    }
}
