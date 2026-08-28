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
public final class CidSchemeHandler: NSObject, WKURLSchemeHandler {
    private let session: PostioSession
    /// The message this web view is showing. Set before each load.
    public var message: Int64?

    public init(session: PostioSession) {
        self.session = session
    }

    public func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard
            let message,
            let url = task.request.url,
            let contentId = Self.contentId(from: url),
            let part = session.resolveCid(message: message, contentId: contentId)
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
        let text = url.absoluteString
        guard text.hasPrefix("\(ReaderConfiguration.cidScheme):") else { return nil }
        var id = String(text.dropFirst(ReaderConfiguration.cidScheme.count + 1))
        while id.hasPrefix("/") { id.removeFirst() }
        return id.removingPercentEncoding.flatMap { $0.isEmpty ? nil : $0 }
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
