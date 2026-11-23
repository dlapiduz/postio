import AppKit
import Testing
import WebKit

@testable import PostioAppKit
@testable import PostioKit

@MainActor
struct MessageBodyTests {
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

    @Test func waitingForThePaintEndsWithThePagesScriptOff() async throws {
        let (view, policy) = try await loaded("<html><body><p>hello</p></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 560, height: 400), styleMask: [.borderless],
            backing: .buffered, defer: false)
        window.contentView = view
        let started = ContinuousClock.now
        try await MessageBodyMeasure.painted(view)
        #expect(ContinuousClock.now - started < .seconds(1))
        window.contentView = nil
        try await MessageBodyMeasure.painted(view)
    }

    @Test func aSentenceTheBodyDoesNotHoldMarksNothing() async throws {
        let (view, policy) = try await loaded("<html><body><p>Nothing to see.</p></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let marked = try await MessageBodyMeasure.highlight(
            "Please leave comments", in: view, fill: "red", line: "red")
        #expect(!marked)
    }

    @Test func aLayoutWiderThanItsBoxIsMeasuredWhole() async throws {
        // The body box scrolls sideways on its own, so the fit must read what the box holds, or a
        // 640 newsletter in a 576 column is cut rather than drawn at 0.9.
        let (view, policy) = try await loaded(
            "<html><body style='margin:0'><div class='postio-body' style='overflow-x:auto'>"
                + "<table width='640'><tr><td>Issue 48</td></tr></table></div></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let width = try await MessageBodyMeasure.layoutWidth(of: view)
        #expect((width ?? 0) >= 640, "measured \(String(describing: width))")
    }

    @Test func theScrollMarkersAreNotTheBodysHeight() async throws {
        // Absolutely placed scroll anchors made scrollHeight thousands of points for a one-line
        // body, which drew nothing.
        let (view, policy) = try await loaded(
            "<html><body style='margin:0'><div class='postio-body'><p style='margin:0;height:40px'>"
                + "One line.</p></div>"
                + "<a id='pos-1' style='position:absolute;top:900vh'></a></body></html>")
        defer { withExtendedLifetime(policy) {} }
        let height = try await MessageBodyMeasure.contentHeight(of: view)
        #expect((height ?? 0) >= 40 && (height ?? .infinity) < 100, "measured \(String(describing: height))")
    }

    @Test func openingAFoldIsToldToTheBodySoItCanGrow() async throws {
        // A `<details>` opens inside the page with script off; without a size report the frame kept
        // the closed height and the quote opened into a clip.
        final class Told { var count = 0 }
        let told = Told()
        let watcher = FoldWatcher { told.count += 1 }
        let configuration = try await ReaderConfiguration.focus(
            cidHandler: ClosedSchemeHandler(),
            fontHandler: FontSchemeHandler { _ in nil },
            baseHandler: ClosedSchemeHandler())
        watcher.install(in: configuration)
        let view = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 560, height: 400), configuration: configuration)
        let policy = ReaderNavigationPolicy(openExternally: { _ in })
        final class Done { var yes = false }
        let done = Done()
        policy.didFinish = { _ in done.yes = true }
        view.navigationDelegate = policy
        defer { withExtendedLifetime(policy) {} }
        view.loadHTMLString(
            "<html><body style='margin:0'><div class='postio-body'><details>"
                + "<summary style='height:20px'>6 quoted lines</summary>"
                + "<blockquote style='margin:0;height:120px'>quoted</blockquote></details></div></body></html>",
            baseURL: nil)
        for _ in 0..<150 where !done.yes { try await Task.sleep(for: .milliseconds(20)) }
        try #require(done.yes)
        let closed = try #require(try await MessageBodyMeasure.contentHeight(of: view))
        _ = try await view.evaluateJavaScript(
            "document.querySelector('details').open = true", in: nil, contentWorld: .defaultClient)
        for _ in 0..<150 where told.count == 0 { try await Task.sleep(for: .milliseconds(20)) }
        #expect(told.count >= 1, "the body was never told a fold opened")
        let opened = try #require(try await MessageBodyMeasure.contentHeight(of: view))
        #expect(opened >= closed + 100, "closed \(closed), opened \(opened)")
    }

    @Test func aWideNewsletterIsZoomedToItsColumnAndNoFurtherThanTheFloor() {
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
