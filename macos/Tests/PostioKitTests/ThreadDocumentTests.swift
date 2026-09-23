import AppKit
import Observation
import PostioFFI
import SwiftUI
import Testing
import WebKit

@testable import PostioKit

extension ReaderWebViews {
    /// The conversation as one document on the Mac (#1595, ADR 0032).
    ///
    /// The page's content is the boundary's and is tested there; the stub here
    /// hands back a small page of the same shape -- one `<details>` per
    /// message, its anchor, its verbs -- so what is asserted is the pane: how
    /// many web views it holds, what it does with a verb, and what it does to
    /// the page when a key asks.
    @MainActor
    @Suite(
        .enabled("lays out a SwiftUI view, so it needs a window server") {
            await MainActor.run { NSScreen.main != nil }
        }
    )
    struct ThreadDocumentTests {
        /// Pages of `count` messages for any thread, numbered from the
        /// thread's id times a hundred, so two threads never share a message.
        final class Pages: ReaderSource, @unchecked Sendable {
            let count: Int
            private let lock = NSLock()
            private var asked = 0
            var documentsAsked: Int { lock.withLock { asked } }

            init(count: Int) { self.count = count }

            func readerDocument(
                message: Int64, remote: RemoteImagesFfi, original: Bool
            ) -> ReaderDocumentFfi {
                ReaderDocumentFfi(html: "", notice: nil, caveat: nil)
            }

            func resolveCid(message: Int64, contentId: String) -> InlinePart? { nil }

            func threadDocument(thread: Int64, originals: [Int64]) -> ThreadDocumentFfi {
                lock.withLock { asked += 1 }
                let ids = (1...Int64(count)).map { thread * 100 + $0 }
                let sections = ids.map { id in
                    """
                    <details class="postio-message" id="m-\(id)" open><summary>Message \(id) \
                    <a href="postio-reply:\(id)">Reply</a></summary><p style="height: 900px">Body \(id)</p></details>
                    """
                }.joined()
                return ThreadDocumentFfi(
                    html: "<!doctype html><html><body>\(sections)</body></html>",
                    messages: ids.map {
                        ThreadAnchorFfi(
                            message: $0, anchor: "m-\($0)", address: "sender\($0)@example.com",
                            caveat: nil
                        )
                    },
                    rail: []
                )
            }
        }

        /// What the pane is asked to show, changed by the tests.
        @MainActor @Observable final class Inputs {
            var thread: Int64 = 1
            var revision = 0
            var request: ConversationModel.DocumentRequest?
        }

        /// What the pane reported.
        @MainActor final class Heard {
            var verbs: [(ThreadVerbFfi, ThreadAnchorFfi?)] = []
            var anchors: [ThreadAnchorFfi] = []
            var observed: [Int64] = []
        }

        struct Host: View {
            let source: any ReaderSource
            let inputs: Inputs
            let heard: Heard

            var body: some View {
                ThreadDocumentView(
                    source: source,
                    thread: inputs.thread,
                    revision: inputs.revision,
                    request: inputs.request,
                    onVerb: { verb, anchor in heard.verbs.append((verb, anchor)) },
                    onAnchors: { heard.anchors = $0 },
                    onObserved: { heard.observed.append($0) }
                )
            }
        }

        private func host(_ source: Pages, _ inputs: Inputs, _ heard: Heard) -> NSWindow {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 800, height: 600),
                styleMask: [.titled], backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: Host(source: source, inputs: inputs, heard: heard)
            )
            window.contentView?.layoutSubtreeIfNeeded()
            return window
        }

        private func turn() async { try? await Task.sleep(for: .milliseconds(10)) }

        /// Take turns until `condition` holds, or give up -- the condition
        /// may ask the page, so it may suspend.
        private func eventually(
            within limit: TimeInterval = 10, _ condition: () async -> Bool
        ) async -> Bool {
            let deadline = Date(timeIntervalSinceNow: limit)
            while !(await condition()) {
                if Date() >= deadline { return false }
                await turn()
            }
            return true
        }

        private func settle() async {
            let until = Date(timeIntervalSinceNow: 0.3)
            while Date() < until { await turn() }
        }

        private func close(_ window: NSWindow, back to: Int64) async {
            window.contentView = nil
            window.close()
            _ = await eventually { readerSurfacesHeld() <= to }
        }

        /// The document's web view, found in the window.
        private func surface(in window: NSWindow) -> ReaderSurface? {
            func find(_ view: NSView) -> ReaderSurface? {
                if let surface = view as? ReaderSurface { return surface }
                for child in view.subviews {
                    if let found = find(child) { return found }
                }
                return nil
            }
            return window.contentView.flatMap(find)
        }

        /// Run `script` in the page and read back a yes or no, if it said one.
        private func evaluate(_ script: String, in view: WKWebView) async -> Bool? {
            await withCheckedContinuation { continuation in
                view.evaluateJavaScript(script) { value, _ in
                    continuation.resume(returning: value as? Bool)
                }
            }
        }

        @Test func aConversationOfThirtyIsOneWebView() async throws {
            // ADR 0032's whole point, on the Mac: the stack held a web view per
            // message it had ever drawn, and scrolling a thirty-message thread
            // ended holding thirty. One page, one process, whatever the length.
            let held = readerSurfacesHeld()
            let created = readerSurfacesCreated()
            let source = Pages(count: 30)
            let heard = Heard()
            let window = host(source, Inputs(), heard)
            try #require(await eventually { heard.anchors.count == 30 })
            await settle()

            #expect(readerSurfacesHeld() - held == 1)
            #expect(readerSurfacesCreated() - created == 1)
            await close(window, back: held)
        }

        @Test func movingToAnotherConversationKeepsTheOneWebView() async throws {
            // The next conversation is the next page in the same view: no
            // process to start, which is the flicker ADR 0032 was written about.
            let held = readerSurfacesHeld()
            let created = readerSurfacesCreated()
            let source = Pages(count: 3)
            let inputs = Inputs()
            let heard = Heard()
            let window = host(source, inputs, heard)
            try #require(await eventually { heard.anchors.first?.message == 101 })
            let renders = readerRendersIssued()

            inputs.thread = 2

            #expect(await eventually { heard.anchors.first?.message == 201 })
            #expect(await eventually { readerRendersIssued() == renders + 1 })
            #expect(readerSurfacesCreated() - created == 1, "a second web view for the second page")
            await close(window, back: held)
        }

        @Test func aReplyInThePageReachesTheVerbWithItsMessage() async throws {
            // Per message (FR-009): the Reply under message 102 answers 102,
            // not the latest -- and never reaches the browser.
            let held = readerSurfacesHeld()
            let heard = Heard()
            let window = host(Pages(count: 3), Inputs(), heard)
            try #require(await eventually { heard.anchors.count == 3 })
            // Scoped: a reference held here would keep the view alive past the
            // teardown below, and the test would wait on itself.
            do {
                let view = try #require(surface(in: window))
                try #require(await eventually(within: 5) { !view.isLoading && view.url != nil })
                _ = await evaluate(
                    "document.querySelector('a[href=\"postio-reply:102\"]').click()", in: view
                )
            }

            #expect(await eventually { !heard.verbs.isEmpty })
            #expect(heard.verbs.first?.0 == ThreadVerbFfi(kind: .reply, message: 102))
            #expect(heard.verbs.first?.1?.address == "sender102@example.com")
            await close(window, back: held)
        }

        @Test func foldingActsOnThePage() async throws {
            let held = readerSurfacesHeld()
            let inputs = Inputs()
            let heard = Heard()
            let window = host(Pages(count: 3), inputs, heard)
            try #require(await eventually { heard.anchors.count == 3 })
            do {
                let view = try #require(surface(in: window))
                try #require(await eventually(within: 5) { !view.isLoading && view.url != nil })
                let open = "document.getElementById('m-101').open"
                try #require(await evaluate(open, in: view) == true)

                inputs.request = ConversationModel.DocumentRequest(
                    action: .toggle(message: 101), serial: 1
                )

                #expect(
                    await eventually { await evaluate(open, in: view) == false },
                    "`z` did nothing to the page"
                )
            }
            await close(window, back: held)
        }

        @Test func anIdenticalPageIsNotLoadedAgain() async throws {
            // A load is a teardown, and the reader's place goes with it. A body
            // arriving for a message already drawn, or any other reason to ask
            // again, must not cost that when the page came out the same --
            // GTK's `would_render_thread`, and #749's fourth cause.
            let held = readerSurfacesHeld()
            let source = Pages(count: 3)
            let inputs = Inputs()
            let heard = Heard()
            let window = host(source, inputs, heard)
            try #require(await eventually { heard.anchors.count == 3 })
            await settle()
            let renders = readerRendersIssued()
            let asked = source.documentsAsked

            inputs.revision += 1

            #expect(await eventually { source.documentsAsked == asked + 1 }, "it was not asked again")
            await settle()
            #expect(readerRendersIssued() == renders, "the same page was loaded twice")
            await close(window, back: held)
        }

        @Test func thePageSaysWhichMessageFillsThePane() async throws {
            // The rail's observer: it speaks once when the page loads, and
            // again when the page settles somewhere else -- from Postio's own
            // content world, since the page's script is off.
            let held = readerSurfacesHeld()
            let heard = Heard()
            let window = host(Pages(count: 4), Inputs(), heard)
            try #require(await eventually { heard.anchors.count == 4 })
            #expect(await eventually { heard.observed.first == 101 }, "\(heard.observed)")

            do {
                let view = try #require(surface(in: window))
                _ = await evaluate(
                    "window.scrollTo(0, document.getElementById('m-103').offsetTop); true", in: view
                )
            }

            #expect(await eventually { heard.observed.last == 103 }, "\(heard.observed)")
            await close(window, back: held)
        }
    }
}

