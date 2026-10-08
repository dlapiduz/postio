import AppKit
import Testing
import WebKit

@testable import PostioAppKit
@testable import PostioKit

/// The message window's body view (specs/009-focus-macos T069): what it
/// asks of a document whose own script is off.
///
/// **Postio's own script still runs.** `allowsContentJavaScript = false` is
/// about the sender's markup; `evaluateJavaScript` and
/// `callAsyncJavaScript` in the client's own content world run regardless,
/// which is what the body's height, the paper's fit and the action
/// sentence's highlight are measured and drawn with. These prove that on
/// the configuration the window uses, rather than assume it.
@MainActor
struct MessageBodyTests {
    /// A hardened Focus web view with `html` loaded, once it has finished.
    private func loaded(_ html: String) async throws -> (WKWebView, ReaderNavigationPolicy) {
        let configuration = try await ReaderConfiguration.focus(
            cidHandler: ClosedSchemeHandler(),
            fontHandler: FontSchemeHandler { _ in nil },
            baseHandler: ClosedSchemeHandler())
        let view = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 560, height: 400), configuration: configuration)
        let policy = ReaderNavigationPolicy(openExternally: { _ in })
        final class Done { var yes = false }
        let done = Done()
        policy.didFinish = { _ in done.yes = true }
        view.navigationDelegate = policy
        view.loadHTMLString(html, baseURL: nil)
        for _ in 0..<150 where !done.yes {
            try await Task.sleep(for: .milliseconds(20))
        }
        try #require(done.yes, "the document never finished loading")
        return (view, policy)
    }

    @Test func postiosOwnScriptRunsWithThePagesOff() async throws {
        let (view, policy) = try await loaded(
            "<html><body><p id=x>hello</p><script>document.title='ran'</script></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let title = try await view.evaluateJavaScript("document.title", in: nil, contentWorld: .defaultClient)
        #expect(title as? String == "", "the page's own script did not run")
        let width = try await MessageBodyMeasure.size(of: view)
        #expect((width?.width ?? 0) > 0, "and the client's measurement did")
    }

    @Test func theActionSentenceIsMarkedInTheBody() async throws {
        let (view, policy) = try await loaded(
            "<html><body><p>Uploaded v3. Please leave comments by Wednesday; I'd like to freeze it.</p></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let marked = try await MessageBodyMeasure.highlight(
            "Please leave comments by Wednesday", in: view, fill: "rgba(0,0,0,0.08)", line: "rgb(0,0,0)")
        #expect(marked)
        let text = try await view.evaluateJavaScript(
            "document.querySelector('mark') && document.querySelector('mark').textContent",
            in: nil, contentWorld: .defaultClient)
        #expect(text as? String == "Please leave comments by Wednesday")
    }

    @Test func aSentenceTheBodyDoesNotHoldMarksNothing() async throws {
        let (view, policy) = try await loaded("<html><body><p>Nothing to see.</p></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let marked = try await MessageBodyMeasure.highlight(
            "Please leave comments", in: view, fill: "red", line: "red")
        #expect(!marked)
    }

    @Test func aLayoutWiderThanItsBoxIsMeasuredWhole() async throws {
        // The reader's body box scrolls sideways on its own (`overflow-x:
        // auto`), so the page's own width is the column's whatever the
        // sender laid out: the fit has to read what the box holds, or a 640
        // newsletter in a 576 column is cut rather than drawn at 0.9.
        let (view, policy) = try await loaded(
            "<html><body style='margin:0'><div class='postio-body' style='overflow-x:auto'>"
                + "<table width='640'><tr><td>Issue 48</td></tr></table></div></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let width = try await MessageBodyMeasure.layoutWidth(of: view)
        #expect((width ?? 0) >= 640, "measured \(String(describing: width))")
    }

    @Test func aWideNewsletterIsZoomedToItsColumnAndNoFurtherThanTheFloor() {
        // SPEC section 3 at 1024: a 640 layout in a 576 column is 0.9.
        #expect(PaperFit.zoom(column: 576, measured: 640, floor: 0.85) == 0.9)
        #expect(PaperFit.zoom(column: 500, measured: 800, floor: 0.85) == 0.85, "the floor")
        #expect(PaperFit.zoom(column: 640, measured: 600, floor: 0.85) == 1, "never enlarged")
        #expect(PaperFit.zoom(column: 640, measured: 0, floor: 0.85) == 1)
    }

    @Test func aFontURLNamesItsFaceAndNothingElse() throws {
        let url = try #require(URL(string: "postio-font:Barlow-Regular.ttf"))
        #expect(FontSchemeHandler.name(from: url) == "Barlow-Regular.ttf")
        let slashed = try #require(URL(string: "postio-font:///Barlow-Bold.ttf"))
        #expect(FontSchemeHandler.name(from: slashed) == "Barlow-Bold.ttf")
        let other = try #require(URL(string: "https://example.com/Barlow.ttf"))
        #expect(FontSchemeHandler.name(from: other) == nil)
    }
}
