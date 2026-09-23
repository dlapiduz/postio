import Foundation
import PostioFFI
import WebKit

/// Answers `postio-cid:` from the local blob store, and nothing else.
///
/// Scoped to one message on purpose. A `Content-ID` is meaningful only inside
/// the message that declares it, so resolving one globally would let a sender's
/// markup address another sender's parts — a crafted `cid:` referencing a
/// colleague's attachment would render it. The message is held here rather
/// than read from ambient state, so it cannot drift from what the view is
/// showing.
///
/// The composer uses it too, over a *draft's* own parts (#1571): the same
/// scoping argument, one message further back. There `message` is the draft
/// being written.
public final class CidSchemeHandler: NSObject, WKURLSchemeHandler {
    private let resolve: (Int64, String) -> InlinePart?
    /// The message this web view is showing — or the draft, in the composer.
    /// Set before each load.
    public var message: Int64?

    /// Every message a conversation document drew (#1595).
    ///
    /// A page holding a thread names each part's message in the reference
    /// itself, and a reference naming a message this page did not draw is
    /// refused rather than resolved: the document is the boundary of what it
    /// may show. GTK's thread reader checks the same way.
    public var scopes: Set<Int64> = []

    /// Over a message's parts: the reader's handler.
    public convenience init(source: any ReaderSource) {
        self.init { message, contentId in
            source.resolveCid(message: message, contentId: contentId)
        }
    }

    /// Over whatever `resolve` answers from, given the scope in `message`
    /// and the `Content-ID` asked for.
    public init(resolve: @escaping (Int64, String) -> InlinePart?) {
        self.resolve = resolve
    }

    public func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard
            let url = task.request.url,
            let reference = Self.reference(from: url),
            let target = reference.scope ?? message,
            reference.scope == nil || reference.scope == message
                || scopes.contains(target),
            let part = resolve(target, reference.contentId)
        else {
            // A miss is an error, not a stall. The `inline-image-cid` corpus
            // fixture is a `cid:` with no matching part and exists to prove
            // the reader shows a broken image rather than waiting for bytes
            // that are never coming.
            task.didFailWithError(
                NSError(domain: NSURLErrorDomain, code: NSURLErrorFileDoesNotExist)
            )
            return
        }

        let response = URLResponse(
            url: url,
            mimeType: part.mimeType,
            expectedContentLength: part.bytes.count,
            textEncodingName: nil
        )
        task.didReceive(response)
        task.didReceive(Data(part.bytes))
        task.didFinish()
    }

    public func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {
        // Nothing to cancel: resolution is synchronous and local, so a task is
        // finished or failed before this could be called.
    }

    /// The `Content-ID` a `postio-cid:` URL addresses.
    ///
    /// Everything after the scheme, without the leading slashes some URL
    /// parsers insert. Kept separate so the parsing is testable — a handler
    /// that mangled the id would resolve nothing and look exactly like a
    /// message whose parts are genuinely absent.
    public static func contentId(from url: URL) -> String? {
        reference(from: url)?.contentId
    }

    /// What a `postio-cid:` URL references: the part, and -- in a document
    /// holding a whole conversation -- which message it belongs to.
    public struct Reference: Equatable, Sendable {
        /// The message, when the page names one; `nil` in a single-message
        /// document, where the message is whichever one is open.
        public let scope: Int64?
        /// The `Content-ID`, decoded.
        public let contentId: String
    }

    /// `postio-cid:<message>/<id>` in a conversation document (ADR 0032),
    /// `postio-cid:<id>` in a single message.
    ///
    /// A raw `/` cannot be part of a content id -- they are percent-encoded,
    /// so `a%2Fb` stays one id -- which is what makes the first one a
    /// separator. What stands before it has to be a message id: the page
    /// names messages by id, and nothing it drew puts anything else there.
    public static func reference(from url: URL) -> Reference? {
        let text = url.absoluteString
        guard text.hasPrefix("\(ReaderConfiguration.cidScheme):") else { return nil }
        var rest = Substring(text.dropFirst(ReaderConfiguration.cidScheme.count + 1))
        while rest.hasPrefix("/") { rest.removeFirst() }
        var scope: Int64?
        if let slash = rest.firstIndex(of: "/") {
            guard let named = Int64(rest[..<slash]) else { return nil }
            scope = named
            rest = rest[rest.index(after: slash)...]
        }
        guard let id = String(rest).removingPercentEncoding, !id.isEmpty else { return nil }
        return Reference(scope: scope, contentId: id)
    }
}

/// Refuses every `postio-reader:` request.
///
/// The reader's document is served by `loadHTMLString(_:baseURL:)`, so nothing
/// ever legitimately fetches this scheme. Registering a handler that always
/// fails is what makes "a relative reference in a sender's markup fails
/// closed" true **by mechanism**: WebKit's behaviour for an unregistered
/// custom-scheme base URL is not specified, and relying on it would be relying
/// on luck.
public final class ClosedSchemeHandler: NSObject, WKURLSchemeHandler {
    public func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        task.didFailWithError(
            NSError(domain: NSURLErrorDomain, code: NSURLErrorUnsupportedURL)
        )
    }

    public func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}
