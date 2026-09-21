import Foundation
import PostioFFI

/// The route from a command id to the composer that has the keyboard.
///
/// # The bug this is
///
/// Fourteen registry commands were drawn in the Message and Format menus,
/// carried a `⌘` chord from the second keyboard layer, were offered in the
/// palette, and reached **nothing**. The composer itself was real the whole
/// time — `ComposeModel` could already apply a mark, attach a file and send.
/// What was missing was anything that turned an id into a call.
/// `postio-gtk`'s composer does it with `connect_command`; this is that, for
/// a frontend whose composer is a window rather than a pane.
///
/// # Why a frontend answers these at all
///
/// Everything else goes to the boundary, where the cursor, the selection and
/// the row window live. A draft being written does not: it is in a window
/// this frontend owns, unsaved, and the store has not seen most of it. So
/// these are `Intercepted`, and the coverage sweep counts them as answered
/// for that reason.
@MainActor
public enum ComposeCommands {
    /// The marks that are a request to the editing surface rather than a call.
    ///
    /// They cannot be applied from here: the document lives in a `WKWebView`
    /// and the mark is script the *surface* runs, so the model records the
    /// request and the view performs it. `postio_ui::compose::mark_script`
    /// owns which script, for both frontends.
    nonisolated private static let marks = [
        "bold", "italic", "bullet_list", "numbered_list", "quote_block",
    ]

    /// Every id this layer claims.
    ///
    /// Held against the registry by `ComposeCommandsTests`: an id here that
    /// is not a command, or is a command the registry does not put in
    /// `Context::Composer`, is a menu item that does the wrong thing.
    nonisolated public static var handled: [String] {
        marks + ["insert_link", "copy_fields", "send", "save_draft", "discard_draft", "attach_file"]
    }

    /// Run `id` against `composer`, and say whether it was the composer's.
    ///
    /// `false` means *not mine* — `archive` in a compose window is the list's
    /// and must be left alone, which is the same bug as the dead verbs in
    /// reverse.
    ///
    /// `session` is optional so the decision can be tested without one: the
    /// ids that need a session say so by doing nothing without it, which is
    /// also what happens in the application before a store has opened.
    @discardableResult
    public static func run(
        _ id: String,
        on composer: ComposeModel,
        through session: PostioSession?
    ) -> Bool {
        if marks.contains(id) {
            composer.applyMark(id)
            return true
        }
        switch id {
        case "insert_link":
            // The address is asked for by the view — a command cannot put up
            // a sheet — so this only says that one is wanted.
            composer.wantsLink = true
        case "copy_fields":
            composer.toggleCopyFields()
        case "send":
            guard let session else { return true }
            _ = composer.send(through: session)
        case "save_draft":
            guard let session else { return true }
            composer.save(through: session)
        case "discard_draft":
            composer.wantsDiscard = true
        case "attach_file":
            composer.wantsAttachment = true
        default:
            return false
        }
        return true
    }
}
