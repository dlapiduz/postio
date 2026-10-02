import AppKit

/// Whether the focused responder takes text, which decides whether a bare
/// character is a binding or a letter.
///
/// `Session.key` takes `in_text_entry` and the boundary's own doc says why it
/// is the caller's to answer: *"a search field that archives mail on `a`
/// reads as a broken application rather than a misrouted key"*. Only the
/// caller can see its own focus.
///
/// # The bug this replaces
///
/// It used to end with
/// `(responder as? NSView)?.window?.fieldEditor(false, for: responder) != nil`,
/// which asks whether the **window** has a field editor — not whether *this*
/// responder is a text field. A window gets one the first time any text field
/// is focused, and Postio's search box lives in the toolbar. From then on the
/// answer was `true` for every responder, so every bare-character binding was
/// refused: `/` did nothing, and so did `j`, `k` and `a`.
///
/// A predicate about a responder must only look at the responder.
///
/// In `PostioKit` rather than beside the monitor because the monitor is in
/// the executable target, where no test can reach it — which is why this went
/// unnoticed.
public enum TypingResponder {
    /// Whether `responder` is something text goes into.
    public static func isTyping(_ responder: NSResponder?) -> Bool {
        guard let responder else { return false }
        // The field editor a focused text field installs, and any other view
        // that accepts text input directly. `NSTextView` is the usual answer.
        if responder is NSTextInputClient { return true }
        // The field itself, in the moment before its editor is installed.
        // `NSTextField` is an `NSControl` and not an input client, so the
        // line above does not cover it.
        if responder is NSTextField { return true }
        return false
    }
}
