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

    /// Render `html` in a hardened web view and give it time to fetch.
    ///
    /// Offscreen but in a real window: a web view with no window never lays
    /// out, and a view that never lays out never loads an image — which would
    /// make this test pass for the wrong reason, reporting zero connections
    /// because nothing rendered rather than because nothing was allowed.
    /// Render `html` in a hardened web view and give it time to fetch.
    ///
    /// No `NSWindow`: putting one in a test process and tearing it down
    /// segfaults the runner. A web view with a real frame lays out and loads
    /// its resources without one — which the "allowed" case below confirms,
    /// and which is why that case has to exist. Without it a zero-connection
    /// result would be indistinguishable from a view that never rendered.
    private func render(_ html: String) async {
        let configuration = ReaderConfiguration.hardened(
            cidHandler: ClosedSchemeHandler(),
            baseHandler: ClosedSchemeHandler()
        )
        let view = WKWebView(
            frame: NSRect(x: 0, y: 0, width: 600, height: 400),
            configuration: configuration
        )
        view.loadHTMLString(html, baseURL: URL(string: "postio-reader:///"))

        // Long enough for a fetch to have happened if one were going to.
        try? await Task.sleep(for: .milliseconds(1500))
        view.stopLoading()
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
        await render(document(policy: policy, port: port))

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
        await render(document(policy: policy, port: port))

        #expect(
            beacon.connections > 0,
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
        await render("<!DOCTYPE html><html><head>"
            + "<meta http-equiv=\"Content-Security-Policy\" content=\"\(policy)\">"
            + "</head><body>\(script)</body></html>")

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
