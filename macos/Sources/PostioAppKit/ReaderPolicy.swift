import AppKit
import PostioFFI
import WebKit
import PostioKit

/// After `loadHTMLString` the only legitimate requests are the two custom schemes; anything else is
/// a sender's markup trying to go somewhere.
@MainActor
public final class ReaderNavigationPolicy: NSObject, WKNavigationDelegate, WKUIDelegate {
    /// A closure so the one place that hands a URL to the system is a single call site the privacy
    /// check can see.
    private let openExternally: (URL) -> Void

    /// A conversation stacks bodies each as tall as its content: a scroll view inside a scroll view
    /// is the shape every mail client that gets this wrong has.
    public var didFinish: ((WKWebView) -> Void)?

    public var onVerb: ((ThreadVerbFfi) -> Void)?

    public init(openExternally: @escaping (URL) -> Void) {
        self.openExternally = openExternally
    }

    public func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        didFinish?(webView)
    }

    /// Async form deliberately: under Swift 6 the completion-handler overload only "nearly matches"
    /// (a warning),
    /// and a nearly-matching delegate method is never called, leaving no navigation policy.
    public func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        preferences: WKWebpagePreferences
    ) async -> (WKNavigationActionPolicy, WKWebpagePreferences) {
        // Again per navigation: the configuration's default covers loads that never reach this
        // delegate.
        preferences.allowsContentJavaScript = false

        switch Self.decide(
            navigationType: navigationAction.navigationType,
            url: navigationAction.request.url
        ) {
        case .allow:
            return (.allow, preferences)
        case .verb(let verb):
            onVerb?(verb)
            return (.cancel, preferences)
        case .openExternally(let url):
            openExternally(url)
            return (.cancel, preferences)
        case .refuse:
            return (.cancel, preferences)
        }
    }

    public func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        nil
    }

    /// Decided without touching AppKit, so the rule that keeps a message from loading a remote page
    /// is testable.
    public enum Decision: Equatable {
        case allow
        case verb(ThreadVerbFfi)
        case openExternally(URL)
        case refuse
    }

    /// Takes the navigation's inputs rather than `WKNavigationAction`, which has no public
    /// initialiser.
    public static func decide(navigationType: WKNavigationType, url: URL?) -> Decision {
        guard let url else { return .refuse }

        // The page's own verbs first, before a click can be read as a link for the browser; matched
        // on a scheme a sender's markup can never carry.
        if let verb = threadVerb(url: url.absoluteString) {
            return .verb(verb)
        }

        // A deliberate click leaves the application, and only a deliberate
        // click. `.linkActivated` is the one navigation type a person caused.
        //
        // POSTIO-CONSENT: reached only when the user activates a link inside a
        // message they are reading. The URL is handed to the system browser
        // and the reading pane does not navigate, so Postio itself makes no
        // request — the browser does, on the user's instruction.
        if navigationType == .linkActivated {
            return .openExternally(url)
        }

        switch url.scheme {
        case ReaderConfiguration.cidScheme:
            return .allow
        case ReaderConfiguration.baseScheme:
            return .allow
        case "about":
            return .allow
        default:
            return .refuse
        }
    }
}
