import Foundation
import PostioFFI

/// `postio://message/<id>`. Parsing is the boundary's (`parse_message_link`). A message link goes
/// to the controller as written (`focus_open_link`); any other `postio:` link is said in the pill
/// as a notice with no Undo; anything else is not routed here. No AppKit (#1264).
public enum PostioLink {
    public static let scheme = "postio"

    public enum Route: Equatable, Sendable {
        case open(uri: String, message: Int64)
        case unknown(String)
    }

    public static func route(_ url: URL) -> Route? {
        guard url.scheme?.lowercased() == scheme else { return nil }
        let uri = url.absoluteString
        guard let message = parseMessageLink(uri: uri) else { return .unknown(linkUnknown()) }
        return .open(uri: uri, message: message)
    }

    public static func toast(_ words: String) -> UiEvent {
        .focusToast(text: words, kind: .notice, undoable: false, seconds: nil)
    }

    /// Links that arrive before a session exists (a captured line launches Postio cold) are held
    /// until the list lands, then taken once in order.
    public struct Waiting: Sendable {
        private var held: [Route] = []

        public init() {}

        public mutating func hold(_ route: Route) {
            held.append(route)
        }

        public mutating func take() -> [Route] {
            defer { held = [] }
            return held
        }
    }
}
