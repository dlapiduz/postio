import AppKit
import PostioKit

/// Looks only at the responder: the old check asked whether the window had a field editor, which is
/// true for every responder once any text field (the toolbar search box) has been focused, so every
/// bare-character binding was refused. In `PostioKit` rather than beside the monitor because the
/// executable target is unreachable by tests.
public enum TypingResponder {
    public static func isTyping(_ responder: NSResponder?) -> Bool {
        guard let responder else { return false }
        if responder is NSTextInputClient { return true }
        // `NSTextField` is an `NSControl`, not an input client, in the moment before its editor is
        // installed.
        if responder is NSTextField { return true }
        return false
    }
}
