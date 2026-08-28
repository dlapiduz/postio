import PostioFFI

/// Where the message list gets its rows.
///
/// A protocol so the table can be driven by something other than a live
/// session. That is not a testing convenience bolted on afterwards: opening a
/// session reads the store's key from the login Keychain, and an
/// ad-hoc-signed build has a new code identity on every rebuild — so a test
/// that needed one would raise a modal dialog on a developer's machine and
/// hang every headless run.
///
/// The *model* behind this is tested in Rust, where it lives: `postio-ffi`
/// asserts the paging, the read-ahead, the bounded cache and the generation
/// guard. What Swift has to get right is narrower — how many rows to claim,
/// what to draw when one is not here yet, and what to reload when it arrives —
/// and none of that needs a real store to be wrong.
public protocol MessageRowSource: AnyObject {
    /// How many rows the list has. Never the length of an array: on the other
    /// side this is a `COUNT`, and a hundred thousand rows are a number rather
    /// than a hundred thousand structs.
    var rowCount: UInt32 { get }

    /// The row at `position`, or `nil` while its page is on its way.
    ///
    /// **Must not block.** This is called for every visible row on every
    /// redraw, on the main thread. `nil` means draw a placeholder; the page is
    /// already being fetched by the time this returns.
    func row(at position: UInt32) -> RowFfi?

    /// The search excerpt for a message, when the list is showing results.
    ///
    /// Defaulted, because a source that only ever lists a folder has nothing
    /// to say here and should not have to write `nil` to say it.
    func snippet(for message: Int64) -> SnippetFfi?
}

public extension MessageRowSource {
    func snippet(for message: Int64) -> SnippetFfi? { nil }
}

/// A row source backed by the engine.
public final class SessionRowSource: MessageRowSource {
    private let session: PostioSession

    public init(session: PostioSession) {
        self.session = session
    }

    public var rowCount: UInt32 { session.rowCount }

    public func row(at position: UInt32) -> RowFfi? { session.row(at: position) }

    public func snippet(for message: Int64) -> SnippetFfi? { session.snippet(for: message) }
}

/// What one row shows, once the decisions are made.
///
/// Separated from the cell so the decisions are testable without AppKit: a
/// row that has not arrived, a sender with no name, a conversation of one that
/// should show no badge. Getting those wrong is invisible in a screenshot and
/// obvious in an assertion.
public struct RowPresentation: Equatable, Sendable {
    /// Who it is from, or a placeholder while the page is in flight.
    public let sender: String
    /// The subject, or a stand-in when the message has none.
    public let subject: String
    /// The snippet under the subject. Empty when there is none.
    public let preview: String
    /// Whether to draw the unread marker.
    public let unread: Bool
    /// Whether to draw the flag.
    public let flagged: Bool
    /// The conversation-size badge, or `nil` when there is nothing to say.
    public let threadBadge: String?
    /// Whether this row is still waiting for its page.
    public let isPlaceholder: Bool

    /// The presentation for a row that has not arrived yet.
    ///
    /// Deliberately not blank: a row of empty strings and a row that is
    /// genuinely empty look identical, and one of them is worth waiting for.
    public static let placeholder = RowPresentation(
        sender: "…",
        subject: "…",
        preview: "",
        unread: false,
        flagged: false,
        threadBadge: nil,
        isPlaceholder: true
    )

    public init(
        sender: String,
        subject: String,
        preview: String,
        unread: Bool,
        flagged: Bool,
        threadBadge: String?,
        isPlaceholder: Bool
    ) {
        self.sender = sender
        self.subject = subject
        self.preview = preview
        self.unread = unread
        self.flagged = flagged
        self.threadBadge = threadBadge
        self.isPlaceholder = isPlaceholder
    }

    /// How a delivered row is drawn.
    public init(row: RowFfi) {
        // A message with no `From` is not a bug to hide: it happens, and
        // "(no sender)" is more honest than a blank column that reads as a
        // rendering failure.
        sender = row.from?.nonEmpty ?? "(no sender)"
        subject = row.subject?.nonEmpty ?? "(no subject)"
        preview = row.preview ?? ""
        unread = !row.seen
        flagged = row.flagged
        // A conversation of one is not a conversation. The badge means "there
        // is more here than this", so at one it says nothing (ADR 0015).
        threadBadge = row.threadCount > 1 ? String(row.threadCount) : nil
        isPlaceholder = false
    }
}

private extension String {
    /// `nil` for a string that is empty or only whitespace.
    var nonEmpty: String? {
        let trimmed = trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? nil : trimmed
    }
}
