import Foundation
import PostioFFI
import WebKit

/// Scoped to one message: a `Content-ID` means something only inside the message that declares it,
/// so resolving one globally would let a sender's markup address another sender's parts. The
/// composer uses it over a draft's own parts (#1571).
public final class CidSchemeHandler: NSObject, WKURLSchemeHandler {
    private let resolve: (Int64, String) -> InlinePart?
    /// Set before each load.
    public var message: Int64?

    /// Every message a conversation document drew (#1595): a reference naming one it did not draw
    /// is refused.
    public var scopes: Set<Int64> = []

    public convenience init(source: any ReaderSource) {
        self.init { message, contentId in
            source.resolveCid(message: message, contentId: contentId)
        }
    }

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
            // A miss is an error, not a stall: the reader shows a broken image rather than wait for
            // bytes that never come.
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
        // Nothing to cancel: resolution is synchronous and local.
    }

    /// Kept separate so parsing is testable: a mangled id would look like a message with genuinely
    /// absent parts.
    public static func contentId(from url: URL) -> String? {
        reference(from: url)?.contentId
    }

    public struct Reference: Equatable, Sendable {
        /// `nil` in a single-message document, where the message is whichever one is open.
        public let scope: Int64?
        public let contentId: String
    }

    /// A raw `/` cannot be in a content id (they are percent-encoded), so the first one is the
    /// separator; what precedes it must be a message id.
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

/// Nothing legitimately fetches this scheme. A handler that always fails makes "fails closed" true
/// by mechanism: WebKit's behaviour for an unregistered custom-scheme base URL is unspecified.
public final class ClosedSchemeHandler: NSObject, WKURLSchemeHandler {
    public func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        task.didFailWithError(
            NSError(domain: NSURLErrorDomain, code: NSURLErrorUnsupportedURL)
        )
    }

    public func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}

/// A web view's content process cannot see the app's faces, so without this a body in app colours
/// silently draws in its own sans. The engine (`reader_font`) answers only for names in its
/// compiled-in table, so a font URL never resolves to a path (ADR 0023).
public final class FontSchemeHandler: NSObject, WKURLSchemeHandler {
    private let resolve: (String) -> Data?

    public init(resolve: @escaping (String) -> Data?) {
        self.resolve = resolve
    }

    public static func name(from url: URL) -> String? {
        let text = url.absoluteString
        let prefix = "\(ReaderConfiguration.fontScheme):"
        guard text.hasPrefix(prefix) else { return nil }
        var rest = Substring(text.dropFirst(prefix.count))
        while rest.hasPrefix("/") { rest.removeFirst() }
        return rest.isEmpty ? nil : String(rest)
    }

    public func webView(_ webView: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url, let name = Self.name(from: url),
              let bytes = resolve(name)
        else {
            task.didFailWithError(NSError(domain: NSURLErrorDomain, code: NSURLErrorFileDoesNotExist))
            return
        }
        task.didReceive(
            URLResponse(url: url, mimeType: "font/ttf", expectedContentLength: bytes.count, textEncodingName: nil))
        task.didReceive(bytes)
        task.didFinish()
    }

    public func webView(_ webView: WKWebView, stop task: WKURLSchemeTask) {}
}
