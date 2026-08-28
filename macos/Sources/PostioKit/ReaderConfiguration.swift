import WebKit

/// The hardened configuration a message body is rendered under.
///
/// Every setting here has a counterpart in `postio-gtk`'s `hardened_settings()`,
/// and the reason each exists is the same: **JavaScript being off does not
/// automatically close the surfaces JavaScript would have used.** WebGL and
/// WebRTC run without a `<script>` tag executing, and the storage APIs persist
/// to disk regardless of whether anything is currently running to read them.
///
/// Built as its own type rather than inline in the view so it can be asserted.
/// A content security policy that is present and permissive looks exactly like
/// one that works, and a `WKWebView` setting that silently stopped applying
/// looks like nothing at all.
public enum ReaderConfiguration {
    /// Where the reader's own document is served from.
    ///
    /// Matches `postio_ui::reader::document::DOCUMENT_BASE_URI`, and a handler
    /// is registered for it that **always fails** — so a relative reference in
    /// a sender's markup fails closed by mechanism rather than by luck.
    public static let baseScheme = "postio-reader"

    /// The scheme inline parts are addressed by.
    public static let cidScheme = "postio-cid"

    /// A configuration with everything scripting-adjacent turned off.
    ///
    /// `@MainActor` because `WKWebViewConfiguration` is: a web view and its
    /// handlers live on the main thread, which is also where a URL scheme
    /// handler is called back — the same constraint the GTK scheme callback
    /// works under, and the reason `resolveCid` is synchronous.
    @MainActor
    public static func hardened(
        cidHandler: WKURLSchemeHandler,
        baseHandler: WKURLSchemeHandler
    ) -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()

        // Nothing this web view does reaches disk. Subsumes the HTML5
        // database, local storage and page cache that the GTK side turns off
        // one at a time.
        configuration.websiteDataStore = .nonPersistent()

        // The headline, and set in two places on purpose: the default applies
        // to navigations that do not go through the policy delegate, and the
        // delegate sets it again per navigation for the ones that do.
        configuration.defaultWebpagePreferences.allowsContentJavaScript = false
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false

        // Media never plays on its own. A message that autoplayed audio would
        // be an advertisement that announced itself to the room.
        configuration.mediaTypesRequiringUserActionForPlayback = .all
        configuration.allowsAirPlayForMediaPlayback = false

        // WebRTC, WebGL and WebAudio have **no public toggles**. JavaScript is
        // off so none of them is reachable, and the document's own
        // `default-src 'none'` closes what is left. That is weaker in
        // *mechanism* than the GTK side, which can turn each off by name, and
        // identical in effect — said here rather than left for a reader to
        // assume parity.
        configuration.preferences.isElementFullscreenEnabled = false

        #if DEBUG
            configuration.preferences.setValue(true, forKey: "developerExtrasEnabled")
        #endif

        configuration.setURLSchemeHandler(cidHandler, forURLScheme: cidScheme)
        configuration.setURLSchemeHandler(baseHandler, forURLScheme: baseScheme)
        return configuration
    }
}
