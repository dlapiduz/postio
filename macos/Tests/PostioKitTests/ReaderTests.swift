import Foundation
import PostioFFI
import Testing
import WebKit

@testable import PostioKit

/// The reader's configuration and navigation policy.
///
/// ADR 0019 Q6 names two readers drifting apart as the highest risk in the
/// port. The *document* is shared Rust and asserted byte-for-byte on that
/// side; what is checked here is the other half — the configuration and the
/// policy, which are Swift's and cannot be seen from Rust at all.
@MainActor
struct ReaderTests {
    private func configuration() -> WKWebViewConfiguration {
        ReaderConfiguration.hardened(
            cidHandler: ClosedSchemeHandler(),
            baseHandler: ClosedSchemeHandler()
        )
    }

    @Test func javaScriptIsOff() {
        // The headline. Everything else in the document's policy assumes it.
        #expect(configuration().defaultWebpagePreferences.allowsContentJavaScript == false)
        #expect(configuration().preferences.javaScriptCanOpenWindowsAutomatically == false)
    }

    @Test func nothingReachesDisk() {
        // Subsumes the HTML5 database, local storage and page cache the GTK
        // side turns off one at a time: a non-persistent store has nowhere to
        // put them. Storage APIs persist regardless of whether anything is
        // running to read them back, so this is not covered by JS being off.
        #expect(configuration().websiteDataStore.isPersistent == false)
    }

    @Test func mediaNeverPlaysOnItsOwn() {
        // A message that autoplayed audio would be an advertisement that
        // announced itself to the room.
        #expect(configuration().mediaTypesRequiringUserActionForPlayback == .all)
        #expect(configuration().allowsAirPlayForMediaPlayback == false)
    }

    @Test func bothCustomSchemesHaveHandlers() {
        // `postio-cid:` resolves inline parts from the local store.
        // `postio-reader:` is the document's own base and must **fail**, so a
        // relative reference in a sender's markup fails closed by mechanism
        // rather than by WebKit's unspecified behaviour for an unregistered
        // scheme.
        let configured = configuration()
        #expect(configured.urlSchemeHandler(forURLScheme: ReaderConfiguration.cidScheme) != nil)
        #expect(configured.urlSchemeHandler(forURLScheme: ReaderConfiguration.baseScheme) != nil)
    }

    @Test func aLinkTheUserClickedLeavesTheApplication() throws {
        // The pane never navigates to a sender's URL. It hands it to the
        // browser and stays where it is.
        let url = try #require(URL(string: "https://example.com/story"))
        let decision = ReaderNavigationPolicy.decide(navigationType: .linkActivated, url: url)
        #expect(decision == .openExternally(url))
    }

    @Test func aRemotePageIsRefusedOutright() throws {
        // Not "opened externally" — refused. A navigation the user did not
        // activate is the sender's markup trying to go somewhere, and the
        // answer is no rather than "open it in their browser instead".
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
        // A handler that mangled the id would resolve nothing and look exactly
        // like a message whose parts are genuinely absent — the kind of bug
        // that gets diagnosed as a sync problem.
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

    // -- a conversation document names each part's message (#1595) ---------

    @Test func aScopedReferenceNamesItsMessageAndItsPart() throws {
        // One page holds a whole thread, so "whichever message is open" names
        // nothing: two messages may each carry a part called `logo`. GTK's
        // handler routes on the same form.
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
        // The page names messages by id. Anything else in that position is a
        // reference nobody this document drew could have written.
        let odd = try #require(URL(string: "postio-cid:elsewhere/logo"))
        #expect(CidSchemeHandler.reference(from: odd) == nil)
    }

    @Test func aVerbInTheDocumentIsTheDocumentsNotTheBrowsers() throws {
        // A per-message Reply is a link the user activates -- and a link the
        // user activates otherwise goes to the browser. The verb has to be
        // recognised first, or every Reply would open a browser tab on a
        // `postio-reply:` URL.
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

/// The scroll wheel reaches the pane behind a message body (user report:
/// "I can't scroll inside a message").
///
/// The conversation stacks bodies inside one scroll view and sizes each web
/// view to its whole document, so a body has nothing of its own to scroll.
/// `WKWebView` consumes every wheel event over its frame anyway, so pointing
/// at a message and scrolling did nothing — only the few points of padding
/// either side of a body still worked, which is not a thing anyone finds.
///
/// Under `ReaderWebViews` because it builds a reader's web view, and every
/// one it builds is counted by the suite beside it.
extension ReaderWebViews {
    @MainActor
    @Suite struct ReaderScrollingTests {
        /// A responder that records what was forwarded to it.
        final class Catcher: NSView {
            var caught = 0
            override func scrollWheel(with event: NSEvent) { caught += 1 }
        }

        @Test func aBodyHandsTheWheelToWhateverIsBehindIt() throws {
            let catcher = Catcher()
            let web = PassingWebView(frame: NSRect(x: 0, y: 0, width: 100, height: 100))
            catcher.addSubview(web)

            // A real scroll event: `NSEvent.mouseEvent` cannot make one — that
            // constructor is for button events and rejects `.scrollWheel`.
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
            // The premise the forwarding rests on. If this clamp ever starts
            // truncating real messages, the forwarding above has to become
            // conditional — so the two are asserted together, on purpose.
            #expect(BodyHeight.clamped(9_000) == 9_000)
            #expect(BodyHeight.maximum >= 12_000)
        }
    }
}
