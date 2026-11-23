import AppKit
import Network
import Testing
import WebKit

@testable import PostioAppKit
@testable import PostioKit

@MainActor
struct ReaderEgressTests {
    final class Beacon {
        private let listener: NWListener
        private let counter = Counter()

        final class Counter: @unchecked Sendable {
            private let lock = NSLock()
            private var value = 0
            func bump() { lock.lock(); value += 1; lock.unlock() }
            var count: Int { lock.lock(); defer { lock.unlock() }; return value }
        }

        init() throws {
            listener = try NWListener(using: .tcp, on: .any)
            let counter = self.counter
            listener.newConnectionHandler = { connection in
                counter.bump()
                connection.cancel()
            }
            listener.start(queue: .global())
        }

        func port() async -> UInt16 {
            for _ in 0..<200 {
                if let port = listener.port?.rawValue, port != 0 { return port }
                try? await Task.sleep(for: .milliseconds(10))
            }
            return 0
        }

        var connections: Int { counter.count }
        func stop() { listener.cancel() }
    }

    @MainActor
    final class Progress {
        var loaded = false
    }

    /// Render `html`, wait for the document to have finished loading, and then
    /// spend a grace period — what the cases proving a *negative* need.
    ///
    /// Requires the load rather than assuming it: a test of "nothing was
    /// fetched" that passes because nothing could have been fetched is the
    /// exact shape that stops protecting anything. Hence `#require` rather
    /// than `#expect` — a run whose document never rendered has proved
    /// nothing, and should say that instead of reporting a zero it came by
    /// honestly.
    ///
    /// POSTIO-FIXED-DEADLINE: the grace period *after* the load is the
    /// subject. An image fetch is dispatched during layout and lands a moment
    /// after the document itself is done, so at that point there is nothing
    /// left to wait *for* — the window is spent in full whatever the result,
    /// to give a fetch that must not happen every chance to happen.
    /// Shortening it would weaken these cases; lengthening it would only make
    /// a passing run slower.
    private func renderCompletely(
        _ html: String, configuration: WKWebViewConfiguration? = nil
    ) async throws {
        let finished = await render(html, configuration: configuration) { $0.loaded }
        try #require(
            finished,
            "the document never finished loading, so a zero-connection result proves nothing"
        )
        try? await Task.sleep(for: .milliseconds(400))
    }

    @discardableResult
    private func render(
        _ html: String,
        configuration given: WKWebViewConfiguration? = nil,
        within limit: Duration = .seconds(15),
        until done: @escaping @MainActor (Progress) -> Bool
    ) async -> Bool {
        let configuration = given ?? ReaderConfiguration.hardened(
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
            // `navigationDelegate` is weak and `policy` is never named again, so ARC could release
            // it and the `didFinish` that proves rendering would never arrive, passing every
            // negative case for the wrong reason.
            withExtendedLifetime(policy) {}
        }
        view.loadHTMLString(html, baseURL: URL(string: "postio-reader:///"))

        let slice = Duration.milliseconds(100)
        var spent = Duration.zero
        while spent < limit {
            if done(progress) { return true }
            try? await Task.sleep(for: slice)
            spent += slice
        }
        return done(progress)
    }

    @Test func aBlockedRemoteImageIsNeverFetched() async throws {
        let beacon = try Beacon()
        defer { beacon.stop() }
        let port = await beacon.port()
        #expect(port != 0, "the beacon never got a port, so this proves nothing")

        let policy = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; "
            + "img-src postio-cid: data:; font-src data:; base-uri 'none'; "
            + "form-action 'none'; frame-src 'none'; connect-src 'none'"
        try await renderCompletely(document(policy: policy, port: port))

        #expect(
            beacon.connections == 0,
            "the reader fetched a remote image while remote images were blocked"
        )
    }

    @Test func anAllowedRemoteImageIsFetched() async throws {
        // Without this, a "zero when blocked" check would pass against a reader that renders no
        // images at all.
        let beacon = try Beacon()
        defer { beacon.stop() }
        let port = await beacon.port()
        #expect(port != 0)

        let policy = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; "
            + "img-src postio-cid: data: http: https:; font-src data:; base-uri 'none'; "
            + "form-action 'none'; frame-src 'none'; connect-src 'none'"
        let fetched = await render(document(policy: policy, port: port)) { _ in
            beacon.connections > 0
        }

        #expect(
            fetched,
            "nothing was fetched even with remote images allowed, so the blocked case above proves nothing"
        )
    }

    @Test func noScriptRunsWhateverTheDocumentSays() async throws {
        // A script that ran could reach the network by a route `img-src` says nothing about.
        let beacon = try Beacon()
        defer { beacon.stop() }
        let port = await beacon.port()
        #expect(port != 0)

        let script = "<script>fetch('http://127.0.0.1:\(port)/via-script')</script>"
        let policy = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; "
            + "img-src postio-cid: data: http: https:; font-src data:; base-uri 'none'; "
            + "form-action 'none'; frame-src 'none'; connect-src 'none'"
        try await renderCompletely(
            "<!DOCTYPE html><html><head>"
                + "<meta http-equiv=\"Content-Security-Policy\" content=\"\(policy)\">"
                + "</head><body>\(script)</body></html>"
        )

        #expect(beacon.connections == 0, "a script ran and reached the network")
    }

    @Test func theMessageWindowFetchesNoRemoteImageEvenWhereThePolicyWouldAllowIt() async throws {
        // The document allows remote images in its CSP, so only the content rule list stands
        // between it and the beacon: the second layer for when the policy is wrong.
        let beacon = try Beacon()
        defer { beacon.stop() }
        let port = await beacon.port()
        #expect(port != 0, "the beacon never got a port, so this proves nothing")

        let policy = "default-src 'none'; script-src 'none'; style-src 'unsafe-inline'; "
            + "img-src postio-cid: data: http: https:; font-src postio-font:; base-uri 'none'; "
            + "form-action 'none'; frame-src 'none'; connect-src 'none'"
        let configuration = try await ReaderConfiguration.focus(
            cidHandler: ClosedSchemeHandler(),
            fontHandler: ClosedSchemeHandler(),
            baseHandler: ClosedSchemeHandler())
        try await renderCompletely(document(policy: policy, port: port), configuration: configuration)

        #expect(
            beacon.connections == 0,
            "the message window fetched a remote image its rule list should have refused"
        )
    }

    private func document(policy: String, port: UInt16) -> String {
        """
        <!DOCTYPE html><html><head>
        <meta http-equiv="Content-Security-Policy" content="\(policy)">
        </head><body>
        <img src="http://127.0.0.1:\(port)/pixel.png" width="10" height="10">
        </body></html>
        """
    }
}
