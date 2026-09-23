import PostioFFI

/// What the reading pane asks the engine for, and all it asks.
///
/// `PostioSession` is the one real answer. The protocol exists so a test can
/// host the real `ReaderView` — and count the web views a conversation makes
/// (#1586) — without a session, which on this platform means the Keychain.
/// Two methods because the reader needs two things: a message's document,
/// and the inline parts that document names.
public protocol ReaderSource: AnyObject, Sendable {
    /// The document for `message`, built by the engine the GTK reader also
    /// calls. Blocks on the store; never call it on the main actor.
    func readerDocument(message: Int64, remote: RemoteImagesFfi, original: Bool) -> ReaderDocumentFfi

    /// The inline part `contentId` names inside `message`, if it is local.
    func resolveCid(message: Int64, contentId: String) -> InlinePart?

    /// `thread` as one document, with each message's anchor (#1595). Blocks
    /// on the store — a body load per message — so never on the main actor.
    func threadDocument(thread: Int64, originals: [Int64]) -> ThreadDocumentFfi
}

extension PostioSession: ReaderSource {}
