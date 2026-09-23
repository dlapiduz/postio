import AppKit
import PostioFFI
import Testing
import WebKit

@testable import PostioKit

/// `space` turns the page, and the proof is that the document moved.
///
/// The most-used key in a mail client did nothing on macOS. Worse than a
/// no-op: `KeyMonitor` returns `true` for any key the resolver claims, so
/// `space` and `Page Down` were *swallowed* and never reached the view
/// underneath either. A long message could not be read past its first screen
/// without the trackpad.
///
/// The mechanism was already there and unused. A hardened reader has no
/// scroll-by-amount call — JavaScript that arrived in the message is off
/// (ADR 0003), and neither WebKit exposes one to the host with it off — so
/// the shared document lays down sixty invisible anchors and a page turn is
/// a same-document jump between them. GTK has jumped between them since it
/// had a reader.
///
/// **This asserts the scroll position, not the call.** The load-bearing
/// question is whether host-evaluated script still runs in a view whose
/// `allowsContentJavaScript` is `false` — a claim about WebKit that is worth
/// proving rather than reading in a document, because if it is wrong the key
/// goes on doing nothing and every other test still passes.
@MainActor
struct ReaderPagingTests {
    @MainActor final class Progress {
        var loaded = false
    }

    /// A document tall enough to page, with the shared anchors in it.
    private func tall() -> String {
        """
        <!doctype html><html><head><meta charset="utf-8">
        <style>body { margin: 0 } .filler { height: 400vh }</style>
        </head><body>\(readerScrollMarkers())<div class="filler">mail</div></body></html>
        """
    }

    private func loaded(_ view: WKWebView, _ policy: ReaderNavigationPolicy, _ progress: Progress)
        async -> Bool
    {
        for _ in 0..<150 {
            if progress.loaded { return true }
            try? await Task.sleep(for: .milliseconds(100))
        }
        withExtendedLifetime(policy) {}
        return progress.loaded
    }

    private func offset(_ view: WKWebView) async -> Double {
        await withCheckedContinuation { continuation in
            view.evaluateJavaScript("window.scrollY") { value, _ in
                continuation.resume(returning: (value as? Double) ?? -1)
            }
        }
    }

    @Test func aPageTurnMovesTheDocumentAndComingBackReturnsIt() async throws {
        let configuration = ReaderConfiguration.hardened(
            cidHandler: ClosedSchemeHandler(),
            baseHandler: ClosedSchemeHandler()
        )
        let view = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 600, height: 400),
            configuration: configuration
        )
        let progress = Progress()
        let policy = ReaderNavigationPolicy(openExternally: { _ in })
        policy.didFinish = { _ in progress.loaded = true }
        view.navigationDelegate = policy
        defer {
            view.stopLoading()
            withExtendedLifetime(policy) {}
        }
        view.loadHTMLString(tall(), baseURL: URL(string: "postio-reader:///"))

        try #require(
            await loaded(view, policy, progress),
            "the document never rendered, so a scroll result proves nothing"
        )
        let start = await offset(view)
        try #require(start >= 0, "host script does not run in this view at all")
        #expect(start == 0, "it did not start at the top: \(start)")

        // Page down twice, through the same helper the application uses.
        var page: UInt32 = 0
        for _ in 0..<2 {
            page = readerPageAfter(current: page, forward: true)
            await ReaderPaging.scroll(view, to: page)
        }
        try? await Task.sleep(for: .milliseconds(300))
        let down = await offset(view)
        #expect(
            down > start,
            "the page key did not move the document: \(start) → \(down)"
        )

        // And back.
        page = readerPageAfter(current: page, forward: false)
        await ReaderPaging.scroll(view, to: page)
        try? await Task.sleep(for: .milliseconds(300))
        let up = await offset(view)
        #expect(up < down, "paging up did not come back: \(down) → \(up)")
    }
}
