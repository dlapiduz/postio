import Foundation
import PostioFFI
import Testing
import WebKit

@testable import PostioAppKit
@testable import PostioKit

@MainActor
struct ReaderTests {
    private func configuration() -> WKWebViewConfiguration {
        ReaderConfiguration.hardened(
            cidHandler: ClosedSchemeHandler(),
            baseHandler: ClosedSchemeHandler()
        )
    }

    @Test func javaScriptIsOff() {
        #expect(configuration().defaultWebpagePreferences.allowsContentJavaScript == false)
        #expect(configuration().preferences.javaScriptCanOpenWindowsAutomatically == false)
    }

    @Test func nothingReachesDisk() {
        // Storage APIs persist regardless of JS, so a non-persistent store is not covered by JS
        // being off.
        #expect(configuration().websiteDataStore.isPersistent == false)
    }

    @Test func mediaNeverPlaysOnItsOwn() {
        #expect(configuration().mediaTypesRequiringUserActionForPlayback == .all)
        #expect(configuration().allowsAirPlayForMediaPlayback == false)
    }

    @Test func bothCustomSchemesHaveHandlers() {
        // `postio-reader:` must fail so a relative reference fails closed by mechanism, not
        // WebKit's unspecified behaviour.
        let configured = configuration()
        #expect(configured.urlSchemeHandler(forURLScheme: ReaderConfiguration.cidScheme) != nil)
        #expect(configured.urlSchemeHandler(forURLScheme: ReaderConfiguration.baseScheme) != nil)
    }

    @Test func aLinkTheUserClickedLeavesTheApplication() throws {
        let url = try #require(URL(string: "https://example.com/story"))
        let decision = ReaderNavigationPolicy.decide(navigationType: .linkActivated, url: url)
        #expect(decision == .openExternally(url))
    }

    @Test func aRemotePageIsRefusedOutright() throws {
        let url = try #require(URL(string: "https://tracker.example/beacon"))
        #expect(ReaderNavigationPolicy.decide(navigationType: .other, url: url) == .refuse)
        #expect(ReaderNavigationPolicy.decide(navigationType: .formSubmitted, url: url) == .refuse)
    }

    @Test func inlinePartsAndTheDocumentBaseAreAllowed() throws {
        let cid = try #require(URL(string: "postio-cid:abc@example.com"))
        let base = try #require(URL(string: "postio-reader:///"))
        #expect(ReaderNavigationPolicy.decide(navigationType: .other, url: cid) == .allow)
        #expect(ReaderNavigationPolicy.decide(navigationType: .other, url: base) == .allow)
    }

    @Test func aNavigationWithNoUrlIsRefused() {
        #expect(ReaderNavigationPolicy.decide(navigationType: .other, url: nil) == .refuse)
    }

    @Test func aContentIdSurvivesTheUrlRoundTrip() throws {
        let simple = try #require(URL(string: "postio-cid:abc@example.com"))
        #expect(CidSchemeHandler.contentId(from: simple) == "abc@example.com")

        let slashed = try #require(URL(string: "postio-cid:///abc@example.com"))
        #expect(CidSchemeHandler.contentId(from: slashed) == "abc@example.com")

        let encoded = try #require(URL(string: "postio-cid:a%2Bb@example.com"))
        #expect(CidSchemeHandler.contentId(from: encoded) == "a+b@example.com")
    }

    @Test func somethingThatIsNotACidUrlYieldsNoId() throws {
        let wrong = try #require(URL(string: "https://example.com/abc"))
        #expect(CidSchemeHandler.contentId(from: wrong) == nil)

        let empty = try #require(URL(string: "postio-cid:"))
        #expect(CidSchemeHandler.contentId(from: empty) == nil)
    }

    @Test func aScopedReferenceNamesItsMessageAndItsPart() throws {
        let scoped = try #require(URL(string: "postio-cid:42/logo"))
        let reference = try #require(CidSchemeHandler.reference(from: scoped))
        #expect(reference.scope == 42)
        #expect(reference.contentId == "logo")
    }

    @Test func aContentIdCannotSmuggleASeparator() throws {
        let encoded = try #require(URL(string: "postio-cid:42/a%2Fb"))
        #expect(CidSchemeHandler.reference(from: encoded)?.contentId == "a/b")
    }

    @Test func anUnscopedReferenceIsWhatItAlwaysWas() throws {
        let plain = try #require(URL(string: "postio-cid:abc@example.com"))
        let reference = try #require(CidSchemeHandler.reference(from: plain))
        #expect(reference.scope == nil)
        #expect(reference.contentId == "abc@example.com")
    }

    @Test func aScopeThatIsNotAMessageResolvesNothing() throws {
        let odd = try #require(URL(string: "postio-cid:elsewhere/logo"))
        #expect(CidSchemeHandler.reference(from: odd) == nil)
    }

    @Test func aVerbInTheDocumentIsTheDocumentsNotTheBrowsers() throws {
        // The reply verb must be recognised before the browser-link path, or every Reply opens a
        // tab on a `postio-reply:` URL.
        let reply = try #require(URL(string: "postio-reply:42"))
        for type in [WKNavigationType.linkActivated, .other] {
            #expect(
                ReaderNavigationPolicy.decide(navigationType: type, url: reply)
                    == .verb(ThreadVerbFfi(kind: .reply, message: 42))
            )
        }
        let web = try #require(URL(string: "https://example.com/"))
        #expect(
            ReaderNavigationPolicy.decide(navigationType: .linkActivated, url: web)
                == .openExternally(web),
            "a sender's own link still goes to the browser"
        )
    }
}

extension ReaderWebViews {
    @MainActor
    @Suite struct ReaderScrollingTests {
        final class Catcher: NSView {
            var caught = 0
            override func scrollWheel(with event: NSEvent) { caught += 1 }
        }

        @Test func aBodyHandsTheWheelToWhateverIsBehindIt() throws {
            let catcher = Catcher()
            let web = PassingWebView(frame: NSRect(x: 0, y: 0, width: 100, height: 100))
            catcher.addSubview(web)

            let scroll = try #require(
                CGEvent(
                    scrollWheelEvent2Source: nil,
                    units: .pixel,
                    wheelCount: 1,
                    wheel1: 10,
                    wheel2: 0,
                    wheel3: 0
                )
            )
            let wheel = try #require(NSEvent(cgEvent: scroll))
            web.scrollWheel(with: wheel)

            #expect(
                catcher.caught == 1,
                "the body swallowed the wheel, so the conversation never scrolls"
            )
        }

        @Test func aBodyIsSizedToItsWholeDocumentSoItHasNothingToScroll() {
            // If this clamp ever truncates real messages the forwarding above must become
            // conditional, so the two are asserted together.
            #expect(BodyHeight.clamped(9_000) == 9_000)
            #expect(BodyHeight.maximum >= 12_000)
        }
    }
}
