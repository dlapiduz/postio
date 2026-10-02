import AppKit
import Network
import Testing
import WebKit

@testable import PostioKit

/// Proof that the reader makes no requests.
///
/// Every other check of that claim is a reading of the code: the settings look
/// right, the policy string looks right, the comment says images are blocked.
/// **A content security policy that is present and permissive looks exactly
/// like one that works**, and a `WKWebView` setting that silently stopped
/// applying looks like nothing at all.
///
/// This is the only assertion that fails when the reader starts fetching.
///
/// It binds a listener on loopback and counts connections. That is not the
/// network in the sense the no-network rule means — nothing leaves this
/// machine, which is precisely the property under test — and it is the same
/// reading the OAuth loopback redirect already relies on.
@MainActor
struct ReaderEgressTests {
    /// A listener on loopback that counts what connects to it.
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

        /// The port, once the listener has one.
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

    /// What a render has got to, for a waiter to look at.
    @MainActor
    final class Progress {
        /// Whether the document has finished loading.
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
    private func renderCompletely(_ html: String) async throws {
        let finished = await render(html) { $0.loaded }
        try #require(
            finished,
            "the document never finished loading, so a zero-connection result proves nothing"
        )
        try? await Task.sleep(for: .milliseconds(400))
    }

    /// Render `html` in a hardened web view and stop as soon as `done()`, or
    /// at the deadline.
    ///
    /// No `NSWindow`: putting one in a test process and tearing it down
    /// segfaults the runner. A web view with a real frame lays out and loads
    /// its resources without one — which the "allowed" case below confirms,
    /// and which is why that case has to exist. Without it a zero-connection
    /// result would be indistinguishable from a view that never rendered.
    ///
    /// # Why this waits on the thing rather than on the clock
    ///
    /// It used to sleep a flat 1500 ms and assert, with no await on the load
    /// at all. That is a race with whatever else the runner is doing, and it
    /// lost one on CI (#1213): on a runner where WebKit's networking took
    /// longer than the window, the fetch had simply not happened *yet*, so a
    /// privacy assertion — "nothing leaves this machine unasked" — failed
    /// having proved nothing. A shared macOS runner is slower and busier than
    /// the desktop that number was chosen on.
    ///
    /// A flat sleep is also wrong in both directions: too short and the
    /// positive case flakes, too long and every run pays for it. So each case
    /// waits for the thing it is actually waiting for — the positive one for
    /// the connection, the negative ones (through `renderCompletely` above)
    /// for the load to finish, and only then for a grace period. The deadline
    /// here is a ceiling rather than a wait, and generous because it costs
    /// nothing when the test passes: a connection that arrives in 200 ms
    /// returns in 200 ms whatever the bound says, and the rest of it is only
    /// ever spent on a run that was going to fail anyway.
    ///
    /// This is the shape `gtk_reader.rs` already uses for the same pair of
    /// claims on Linux — `wait_for` on a tracked load, then `pump_for` for the
    /// blocked case, and `wait_for_connection` for the allowed one.
    @discardableResult
    private func render(
        _ html: String,
        within limit: Duration = .seconds(15),
        until done: @escaping @MainActor (Progress) -> Bool
    ) async -> Bool {
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
            // `navigationDelegate` is a weak reference and `policy` is never
            // named again after it is assigned, so ARC is free to release it
            // immediately — and then the `didFinish` that proves the document
            // rendered would never arrive, and every negative case would pass
            // for the one reason that makes it worthless.
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

        // The policy the engine produces for a blocked message, verbatim.
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
        // The other half, and the reason the first assertion means anything. A
        // test that only checked "zero when blocked" would pass against a
        // reader that renders no images at all, which is not the property
        // being claimed.
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
        // JavaScript is off in the configuration, and the document's own
        // policy says `script-src 'none'`. A script that ran could reach the
        // network by a route `img-src` says nothing about.
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
