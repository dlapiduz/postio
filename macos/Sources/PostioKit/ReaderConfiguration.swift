import WebKit

/// Each setting closes a surface that JavaScript being off does not: WebGL and WebRTC run without a
/// script, and storage APIs persist. A type of its own so it can be asserted.
public enum ReaderConfiguration {
    /// Matches `postio_ui::reader::document::DOCUMENT_BASE_URI`; its handler always fails, so a
    /// relative reference in a sender's markup fails closed.
    public static let baseScheme = "postio-reader"

    public static let cidScheme = "postio-cid"

    /// `postio_ui::reader::document::FONT_SCHEME` (ADR 0023).
    public static let fontScheme = "postio-font"

    /// `@MainActor` because `WKWebViewConfiguration` is, and scheme handlers are called back there,
    /// which is why `resolveCid` is synchronous.
    @MainActor
    public static func hardened(
        cidHandler: WKURLSchemeHandler,
        baseHandler: WKURLSchemeHandler
    ) -> WKWebViewConfiguration {
        let configuration = WKWebViewConfiguration()

        // Nothing this web view does reaches disk.
        configuration.websiteDataStore = .nonPersistent()

        // Set in two places on purpose: this default covers navigations that skip the policy
        // delegate, which sets it again per navigation.
        configuration.defaultWebpagePreferences.allowsContentJavaScript = false
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false

        // Media never plays on its own.
        configuration.mediaTypesRequiringUserActionForPlayback = .all
        configuration.allowsAirPlayForMediaPlayback = false

        // WebRTC, WebGL and WebAudio have no public toggles: JavaScript off makes them unreachable
        // and the document's `default-src 'none'` closes the rest. Weaker in mechanism than GTK,
        // identical in effect.
        configuration.preferences.isElementFullscreenEnabled = false

        #if DEBUG
            configuration.preferences.setValue(true, forKey: "developerExtrasEnabled")
        #endif

        configuration.setURLSchemeHandler(cidHandler, forURLScheme: cidScheme)
        configuration.setURLSchemeHandler(baseHandler, forURLScheme: baseScheme)
        return configuration
    }

    /// `hardened`, Postio's faces over `postio-font:`, and `blockingRules`: a second wall behind
    /// the document's CSP.
    @MainActor
    public static func focus(
        cidHandler: WKURLSchemeHandler,
        fontHandler: WKURLSchemeHandler,
        baseHandler: WKURLSchemeHandler
    ) async throws -> WKWebViewConfiguration {
        let configuration = hardened(cidHandler: cidHandler, baseHandler: baseHandler)
        configuration.setURLSchemeHandler(fontHandler, forURLScheme: fontScheme)
        configuration.userContentController.add(try await blockingRules())
        return configuration
    }

    /// Postio's three, the `data:` images and faces a sanitised body keeps, and `about:`, which a
    /// document loaded from a string with no base has.
    public static let allowedSchemes = [cidScheme, fontScheme, baseScheme, "data", "about"]

    /// One rule per scheme: WebKit's rule regexes have no alternation, so `^(a|b):` would not
    /// compile.
    public static var blockingRuleSource: String {
        let allow = allowedSchemes.map {
            #"{"trigger":{"url-filter":"^\#($0):"},"action":{"type":"ignore-previous-rules"}}"#
        }
        let block = #"{"trigger":{"url-filter":".*"},"action":{"type":"block"}}"#
        return "[" + ([block] + allow).joined(separator: ",") + "]"
    }

    static let blockingRulesIdentifier = "postio.message.block-remote"

    /// Compiled into a store of its own in the temporary directory, not WebKit's default: nothing
    /// about a filter needs to outlive the process.
    @MainActor
    public static func blockingRules() async throws -> WKContentRuleList {
        if let compiled = compiledRules { return compiled }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("postio-content-rules", isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        guard let store = WKContentRuleListStore(url: directory) else {
            throw CocoaError(.fileWriteUnknown)
        }
        let list = try await store.compileContentRuleList(
            forIdentifier: blockingRulesIdentifier, encodedContentRuleList: blockingRuleSource)
        guard let list else { throw CocoaError(.coderInvalidValue) }
        compiledRules = list
        return list
    }

    @MainActor private static var compiledRules: WKContentRuleList?
}
