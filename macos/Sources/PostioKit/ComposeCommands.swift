import Foundation
import PostioFFI

/// Registry commands drawn in the Message and Format menus reached nothing until an id turned into
/// a call. A draft is in a
/// window this frontend owns, unsaved and unknown to the store, so these are `Intercepted` and the
/// coverage sweep counts them as answered.
/// The four verbs that write: the controller's, named for the toolbar's compose button and its tip.
public enum WritingCommand {
    public static let compose = "compose"
    public static let reply = "reply"
    public static let replyAll = "reply_all"
    public static let forward = "forward"
}

@MainActor
public enum ComposeCommands {
    /// The document lives in a `WKWebView` and the mark is script the surface runs, so the model
    /// records the request and the view performs it (`postio_ui::compose::mark_script`).
    nonisolated private static let marks = [
        "bold", "italic", "heading_1", "bullet_list", "numbered_list", "quote_block",
    ]

    /// Held against the registry by `ComposeCommandsTests`: an id that is not a Composer command is
    /// a menu item that does the wrong thing.
    nonisolated public static var handled: [String] {
        marks + [
            "insert_link", "copy_fields", "send", "save_draft", "discard_draft",
            "attach_file", "schedule_send", "insert_image", "remind_if_no_reply",
        ]
    }

    /// `false` means not mine: `archive` in a compose window is the list's and must be left alone.
    /// `session` is optional so the decision is testable; ids that need one do nothing without it.
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
            // A command cannot put up a sheet: this only says one is wanted.
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
        case "schedule_send":
            composer.wantsSchedule = true
        case "remind_if_no_reply":
            composer.wantsRemind = true
        case "insert_image":
            composer.askForImage()
        default:
            return false
        }
        return true
    }
}
