import PostioFFI

/// The protocol lets a test host the real `ReaderView` and count web views (#1586) without a
/// session, which here means the Keychain.
public protocol ReaderSource: AnyObject, Sendable {
    /// Blocks on the store; never call it on the main actor.
    func readerDocument(message: Int64, remote: RemoteImagesFfi, reduced: Bool) -> ReaderDocumentFfi

    func resolveCid(message: Int64, contentId: String) -> InlinePart?

    /// Blocks on the store (a body load per message); never on the main actor.
    func threadDocument(thread: Int64, reduced: [Int64]) -> ThreadDocumentFfi
}

extension PostioSession: ReaderSource {}
