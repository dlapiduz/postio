import AppKit
import PostioFFI
import SwiftUI
import WebKit

/// A whole conversation in one web view (ADR 0032, #1595).
///
/// The stack this replaces drew a web view per open message, and a web view
/// is a content process: scrolling a thirty-message thread ended holding
/// thirty, and every one a lazy stack had drawn stayed held until the
/// conversation changed (`docs/notes/2026-09-22-a-lazy-stack-hides-what-it-
/// stops-drawing.md`). Here the thread is one page, composed by the boundary
/// with `postio_ui::reader::thread` — the same page GTK's pane draws — and one
/// view shows it, whatever its length. The next conversation is the next page
/// in the same view, so moving between them starts no process at all.
///
/// # What is the page's and what is not
///
/// Each message's own chrome — sender, date, recipients, `Reply`, `Forward`,
/// `Show` for its images — is in the page, and its verbs are links this view
/// intercepts by scheme before anything can reach the browser. The
/// conversation's header and the notices about the message the list is on
/// stay native, around this view, as GTK's stay GTK.
///
/// # The page is the state
///
/// With the page's script off, the application cannot see a person open or
/// shut a message. So folding, expanding and moving between messages are
/// requests *to the page* (`ConversationModel.DocumentRequest`), carried out
/// with Postio's own host-evaluated script from the boundary.
public struct ThreadDocumentView: NSViewRepresentable {
    private let source: any ReaderSource
    private let thread: Int64
    private let originals: [Int64]
    /// Bumped when what the page is made of may have changed underneath it —
    /// a body arrived, a grant was made. The page is asked for again and
    /// loaded only if it came out different.
    private let revision: Int
    /// The message to land on when a conversation opens (FR-015).
    private let focus: Int64?
    private let request: ConversationModel.DocumentRequest?
    /// Paging, as `ReaderView` pages: a jump between the shared anchors.
    private let page: UInt32
    private let pageToken: Int
    private let onVerb: (ThreadVerbFfi, ThreadAnchorFfi?) -> Void
    private let onAnchors: ([ThreadAnchorFfi]) -> Void
    /// The rail's rows, from the page that was drawn.
    private let onRail: ([RailRowFfi]) -> Void
    /// The page reports which message fills the pane -- the rail's observer.
    private let onObserved: (Int64) -> Void
    /// A scroll the rail asked for has arrived; its token goes back.
    private let onSettled: (UInt64) -> Void

    public init(
        source: any ReaderSource,
        thread: Int64,
        originals: [Int64] = [],
        revision: Int = 0,
        focus: Int64? = nil,
        request: ConversationModel.DocumentRequest? = nil,
        page: UInt32 = 0,
        pageToken: Int = 0,
        onVerb: @escaping (ThreadVerbFfi, ThreadAnchorFfi?) -> Void,
        onAnchors: @escaping ([ThreadAnchorFfi]) -> Void = { _ in },
        onRail: @escaping ([RailRowFfi]) -> Void = { _ in },
        onObserved: @escaping (Int64) -> Void = { _ in },
        onSettled: @escaping (UInt64) -> Void = { _ in }
    ) {
        self.source = source
        self.thread = thread
        self.originals = originals
        self.revision = revision
        self.focus = focus
        self.request = request
        self.page = page
        self.pageToken = pageToken
        self.onVerb = onVerb
        self.onAnchors = onAnchors
        self.onRail = onRail
        self.onObserved = onObserved
        self.onSettled = onSettled
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(source: source)
    }

    public func makeNSView(context: Context) -> ReaderSurface {
        let coordinator = context.coordinator
        coordinator.onVerb = onVerb
        coordinator.onAnchors = onAnchors
        coordinator.onRail = onRail
        coordinator.onObserved = onObserved
        coordinator.onSettled = onSettled
        let configuration = ReaderConfiguration.hardened(
            cidHandler: coordinator.cid,
            baseHandler: coordinator.closed
        )
        // The rail's observer reports in Postio's own content world, where
        // the page's script -- off -- does not reach and the sender's markup
        // cannot post to it.
        configuration.userContentController.add(
            coordinator.reports, contentWorld: .defaultClient, name: Coordinator.railHandler
        )
        // Not `PassingWebView`: this page scrolls itself. It is the pane.
        let view = ReaderSurface(frame: .zero, configuration: configuration)
        view.navigationDelegate = coordinator.policy
        view.uiDelegate = coordinator.policy
        view.setValue(false, forKey: "drawsBackground")
        view.allowsBackForwardNavigationGestures = false
        view.setAccessibilityRole(.group)
        view.setAccessibilityRoleDescription("article")
        view.setAccessibilityLabel(Pane.reader.label)
        // Whatever was asked before this view existed is not a request to it.
        coordinator.performed = request?.serial ?? 0
        coordinator.load(
            into: view, thread: thread, originals: originals, revision: revision, focus: focus
        )
        return view
    }

    public func updateNSView(_ view: ReaderSurface, context: Context) {
        let coordinator = context.coordinator
        coordinator.onVerb = onVerb
        coordinator.onAnchors = onAnchors
        coordinator.onRail = onRail
        coordinator.onObserved = onObserved
        coordinator.onSettled = onSettled
        coordinator.load(
            into: view, thread: thread, originals: originals, revision: revision, focus: focus
        )
        coordinator.perform(request, in: view)
        coordinator.page(view, to: page, token: pageToken)
    }

    /// Holds the handlers, the page on screen, and where it came from.
    @MainActor
    public final class Coordinator {
        let cid: CidSchemeHandler
        let closed = ClosedSchemeHandler()
        let policy: ReaderNavigationPolicy
        var onVerb: (ThreadVerbFfi, ThreadAnchorFfi?) -> Void = { _, _ in }
        var onAnchors: ([ThreadAnchorFfi]) -> Void = { _ in }
        var onRail: ([RailRowFfi]) -> Void = { _ in }
        var onObserved: (Int64) -> Void = { _ in }
        var onSettled: (UInt64) -> Void = { _ in }
        /// Where the observer's reports arrive.
        let reports = RailReports()
        /// The channel's name, as the observer script is told it.
        static let railHandler = "postioRail"
        /// The last request carried out, so SwiftUI's repeated updates do not
        /// fold a message back and forth.
        var performed = 0

        private let source: any ReaderSource
        private let gate = RenderGate()
        private var pending: Task<Void, Never>?
        private var pagedTo: Int?
        /// What was last asked for, so an update for any other reason asks
        /// nothing.
        private var asked: Asked?
        /// The page on screen, so the same page is not loaded twice.
        private var showing: String?
        /// Where each message is in the page on screen.
        private var anchors: [ThreadAnchorFfi] = []
        /// A message to scroll to once the page that holds it has loaded.
        private var landing: String?

        private struct Asked: Equatable {
            let thread: Int64
            let originals: [Int64]
            let revision: Int
        }

        init(source: any ReaderSource) {
            self.source = source
            cid = CidSchemeHandler { message, contentId in
                source.resolveCid(message: message, contentId: contentId)
            }
            let policy = ReaderNavigationPolicy { url in
                // POSTIO-CONSENT: only from a link the user activated inside a
                // message they are reading. The pane does not navigate; the
                // URL goes to whatever the user has chosen as their browser.
                NSWorkspace.shared.open(url)
            }
            self.policy = policy
            policy.onVerb = { [weak self] verb in
                guard let self else { return }
                self.onVerb(verb, self.anchors.first { $0.message == verb.message })
            }
            policy.didFinish = { [weak self] view in
                guard let self else { return }
                if let anchor = self.landing {
                    self.landing = nil
                    view.evaluateJavaScript(threadScrollScript(anchor: anchor))
                }
                // Every load is a new page with no listener on it. Installed
                // after the landing scroll, so its first report is where the
                // pane landed.
                if self.anchors.count > 1 {
                    view.evaluateJavaScript(
                        threadObserverScript(handler: Self.railHandler), in: nil, in: .defaultClient
                    )
                }
            }
            reports.coordinator = self
        }

        /// A report from the observer, as a message id.
        ///
        /// Untrusted input even though Postio wrote the script: it arrives
        /// from a page holding several senders' markup. Only a message this
        /// page drew is passed on.
        fileprivate func observedMessage(_ message: Int64) {
            guard anchors.contains(where: { $0.message == message }) else { return }
            onObserved(message)
        }

        /// Ask for the page, off the main actor, and load it if it changed.
        ///
        /// Results can arrive out of order — a person moving through the list
        /// faster than bodies load — so each carries a token and a stale one
        /// is dropped rather than drawn: one conversation's page under
        /// another's header is the shape of #70.
        func load(
            into view: WKWebView,
            thread: Int64,
            originals: [Int64],
            revision: Int,
            focus: Int64?
        ) {
            let now = Asked(thread: thread, originals: originals, revision: revision)
            guard now != asked else { return }
            let opening = asked?.thread != thread
            asked = now

            let token = gate.begin()
            pending?.cancel()
            let source = self.source
            // `view` weakly, like `self`: a pane SwiftUI has let go of is not
            // the render's to keep (#1586).
            pending = Task { [weak self, weak view] in
                let document = await Task.detached {
                    source.threadDocument(thread: thread, originals: originals)
                }.value
                guard let self, let view, !Task.isCancelled, self.gate.isCurrent(token) else {
                    return
                }
                self.anchors = document.messages
                // Only the messages this page drew may have their parts
                // resolved through it.
                self.cid.scopes = Set(document.messages.map(\.message))
                self.onAnchors(document.messages)
                self.onRail(document.rail)
                if opening {
                    self.landing = document.messages.first { $0.message == focus }?.anchor
                }
                // An identical page is not loaded again: a load is a teardown,
                // and the reader's place in the conversation goes with it.
                // GTK's `would_render_thread`, and #749's fourth cause.
                guard document.html != self.showing else { return }
                self.showing = document.html
                // The one load choke point, so the one place a render is
                // counted -- the counter GTK's reader notes into.
                noteReaderRender()
                view.loadHTMLString(
                    document.html,
                    baseURL: URL(string: "\(ReaderConfiguration.baseScheme):///")
                )
            }
        }

        /// Carry out what a key asked of the page, once.
        func perform(_ request: ConversationModel.DocumentRequest?, in view: WKWebView) {
            guard let request, request.serial != performed else { return }
            performed = request.serial
            guard let script = script(for: request.action) else { return }
            let settle = request.settle
            // The scroll is instant, so its completion is its arrival -- the
            // moment the rail may listen to the observer again.
            view.evaluateJavaScript(script) { [weak self] _, _ in
                guard let self, let settle else { return }
                self.onSettled(settle)
            }
        }

        /// The boundary's script for `action`, against the page on screen.
        ///
        /// `nil` for a message this page does not hold, which is a key that
        /// arrived for the last conversation: there is nothing here for it.
        private func script(for action: ConversationModel.DocumentAction) -> String? {
            switch action {
            case let .scrollTo(message, _):
                return anchor(of: message).map { threadScrollScript(anchor: $0) }
            case let .toggle(message):
                return anchor(of: message).map { threadToggleScript(anchor: $0) }
            case .expandAll:
                return threadExpandAllScript()
            }
        }

        private func anchor(of message: Int64) -> String? {
            anchors.first { $0.message == message }?.anchor
        }

        /// Jump to scroll anchor `page`, once per `token`. See `ReaderView`.
        func page(_ view: WKWebView, to page: UInt32, token: Int) {
            guard pagedTo != token else { return }
            pagedTo = token
            guard token != 0 else { return }
            Task { @MainActor in await ReaderPaging.scroll(view, to: page) }
        }
    }
}

/// Receives the rail observer's reports and hands them to the pane's
/// coordinator, weakly: a content controller keeps its handlers alive, and a
/// strong one here would keep the coordinator alive with it.
@MainActor
public final class RailReports: NSObject, WKScriptMessageHandler {
    weak var coordinator: ThreadDocumentView.Coordinator?

    public func userContentController(
        _ controller: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard let scope = message.body as? String, let id = Int64(scope) else { return }
        coordinator?.observedMessage(id)
    }
}
