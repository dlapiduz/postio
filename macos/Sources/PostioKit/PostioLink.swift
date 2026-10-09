import Foundation
import PostioFFI

/// Postio's own links, `postio://message/<id>` (specs/009-focus-macos
/// T117, C21): what a captured task line carries back to its message.
///
/// The reading is the boundary's (`parse_message_link`, over
/// `postio_ui::links`), so a link reads here exactly as capture wrote it.
/// What this decides is only where a link goes:
///
/// - a link that names a message goes to the controller as written
///   (`focus_open_link`), which looks the message up and opens it in the
///   message window, or says `link_gone` in the pill when it is no longer
///   on this computer;
/// - any other `postio:` link is said in the pill at once, in the
///   boundary's words (`link_unknown`) and as the controller's own refusal
///   is drawn -- a notice, with no Undo -- so the two cannot read
///   differently;
/// - anything else is not Postio's to route here (`mailto:` is `Mailto`'s).
///
/// No AppKit: the application delegate hands it a `URL`, and iOS must stay
/// reachable (#1264).
public enum PostioLink {
    /// The scheme, as `Info.plist` registers it.
    public static let scheme = "postio"

    /// Where one link goes.
    public enum Route: Equatable, Sendable {
        /// To the controller: `focus_open_link(uri)`.
        case open(uri: String, message: Int64)
        /// To the pill, saying this.
        case unknown(String)
    }

    /// Where `url` goes, or `nil` when it is not a `postio:` link.
    public static func route(_ url: URL) -> Route? {
        guard url.scheme?.lowercased() == scheme else { return nil }
        let uri = url.absoluteString
        guard let message = parseMessageLink(uri: uri) else { return .unknown(linkUnknown()) }
        return .open(uri: uri, message: message)
    }

    /// `words` as the pill draws them: the event the controller's own
    /// refusal of a link arrives as (`ToastKind::Notice`, no Undo, its
    /// usual seconds).
    public static func toast(_ words: String) -> UiEvent {
        .focusToast(text: words, kind: .notice, undoable: false, seconds: nil)
    }

    /// Links that arrived before there was a session to open them in --
    /// clicking a captured line launches Postio cold -- held until the list
    /// has landed, then each taken once, in the order they came.
    public struct Waiting: Sendable {
        private var held: [Route] = []

        public init() {}

        /// Keep `route` for later.
        public mutating func hold(_ route: Route) {
            held.append(route)
        }

        /// Everything held, oldest first; nothing is held after.
        public mutating func take() -> [Route] {
            defer { held = [] }
            return held
        }
    }
}
