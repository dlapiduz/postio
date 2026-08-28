import AppKit
import WebKit

/// What the reader is allowed to navigate to, which is almost nothing.
///
/// The document is loaded once with `loadHTMLString`; after that the only
/// legitimate requests are the two custom schemes. Everything else is a
/// sender's markup trying to go somewhere, and the answer is no.
@MainActor
public final class ReaderNavigationPolicy: NSObject, WKNavigationDelegate, WKUIDelegate {
    /// Called with a URL the user activated, so the caller can open it.
    ///
    /// A closure rather than opening it here, so the one place in this
    /// application that hands a URL to the system is a single call site the
    /// privacy check can see.
    private let openExternally: (URL) -> Void

    public init(openExternally: @escaping (URL) -> Void) {
        self.openExternally = openExternally
    }

    /// The async form, deliberately.
    ///
    /// The completion-handler overload "nearly matches" under Swift 6 — the
    /// handler is `@MainActor @Sendable` and a plain `@escaping` closure is a
    /// different type. The compiler says so as a *warning*, and the
    /// consequence is not cosmetic: a delegate method that nearly matches is
    /// never called, so the reader would have had no navigation policy at all
    /// while looking entirely correct.
    public func webView(
        _ webView: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction,
        preferences: WKWebpagePreferences
    ) async -> (WKNavigationActionPolicy, WKWebpagePreferences) {
        // Again, per navigation. The configuration's default covers loads that
        // never reach this delegate; this covers the ones that do, and one
        // place where it could be forgotten is one too many.
        preferences.allowsContentJavaScript = false

        switch Self.decide(
            navigationType: navigationAction.navigationType,
            url: navigationAction.request.url
        ) {
        case .allow:
            return (.allow, preferences)
        case .openExternally(let url):
            openExternally(url)
            return (.cancel, preferences)
        case .refuse:
            return (.cancel, preferences)
        }
    }

    /// A sender's markup asking for a new window gets none.
    public func webView(
        _ webView: WKWebView,
        createWebViewWith configuration: WKWebViewConfiguration,
        for navigationAction: WKNavigationAction,
        windowFeatures: WKWindowFeatures
    ) -> WKWebView? {
        nil
    }

    /// What to do with a navigation, decided without touching AppKit.
    ///
    /// Separated so the policy is testable: this is the rule that keeps a
    /// message from loading a remote page inside the reading pane, and it
    /// would otherwise only be checkable by rendering hostile mail.
    public enum Decision: Equatable {
        /// The reader's own document, or one of its two schemes.
        case allow
        /// A link the user activated: hand it to the browser, do not navigate.
        case openExternally(URL)
        /// Anything else.
        case refuse
    }

    /// Takes the navigation's *inputs* rather than the `WKNavigationAction`
    /// itself, because that type has no public initialiser — a rule that could
    /// only be checked by rendering hostile mail would not be checked at all.
    public static func decide(navigationType: WKNavigationType, url: URL?) -> Decision {
        guard let url else { return .refuse }

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
            // Inline parts, answered from the local blob store.
            return .allow
        case ReaderConfiguration.baseScheme:
            // The document's own base. The handler refuses it anyway; allowing
            // the navigation only means the refusal comes from one place.
            return .allow
        case "about":
            // `about:blank`, which is what `loadHTMLString` navigates to.
            return .allow
        default:
            return .refuse
        }
    }
}
