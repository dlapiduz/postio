import AppKit
import PostioFFI
import SwiftUI
import WebKit

/// The reading pane.
///
/// Its entire job is to build a hardened configuration, hand it a string, and
/// refuse navigations. The content security policy, the embedded font faces,
/// the sanitized body inside its container and the scroll markers all arrive
/// from the engine as one document — the same one the GTK reader renders, from
/// the same function. The two readers do not *agree* on the policy; there is
/// one that produces it.
public struct ReaderView: NSViewRepresentable {
    private let source: any ReaderSource
    private let message: Int64?
    private let remoteImages: RemoteImagesFfi
    /// Whether to draw what the sender wrote rather than what reader view
    /// reduces it to.
    private let original: Bool
    private let onHeight: ((CGFloat) -> Void)?
    /// Called with the two facts the render already paid for — the notice
    /// and the caveat (#1589). They used to be their own boundary calls,
    /// each re-loading the body this render loads anyway.
    private let onAnswers: ((ReaderNoticeFfi?, String?) -> Void)?
    /// Which scroll anchor to sit on, and a token that changes every time it
    /// is asked for.
    ///
    /// A hardened web view has no scroll-by-amount call — script that arrived
    /// in the message is off (ADR 0003) — so paging is a jump between the
    /// anchors the shared document lays down. See `ReaderPaging`.
    ///
    /// The token, not the page: paging down at the last anchor leaves the
    /// number where it was, and a view watching the value alone would not
    /// act on the press.
    private let page: UInt32
    private let pageToken: Int

    /// `onHeight` is how a *stacked* reader is drawn: inside a conversation
    /// the pane scrolls and each body is sized to its content, so the height
    /// has to be measured from the laid-out document. Left `nil` the view
    /// fills whatever it is given, which is what a single-message pane wants.
    public init(
        source: any ReaderSource,
        message: Int64?,
        remoteImages: RemoteImagesFfi = .blocked,
        original: Bool = false,
        page: UInt32 = 0,
        pageToken: Int = 0,
        onHeight: ((CGFloat) -> Void)? = nil,
        onAnswers: ((ReaderNoticeFfi?, String?) -> Void)? = nil
    ) {
        self.source = source
        self.message = message
        self.remoteImages = remoteImages
        self.original = original
        self.page = page
        self.pageToken = pageToken
        self.onHeight = onHeight
        self.onAnswers = onAnswers
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(source: source, onHeight: onHeight, onAnswers: onAnswers)
    }

    public func makeNSView(context: Context) -> WKWebView {
        let coordinator = context.coordinator
        let configuration = ReaderConfiguration.hardened(
            cidHandler: coordinator.cid,
            baseHandler: coordinator.closed
        )
        let view = PassingWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = coordinator.policy
        view.uiDelegate = coordinator.policy
        view.setValue(false, forKey: "drawsBackground")
        // The pane is Postio's chrome, not a browser: no rubber-banding past
        // the document, and no back-forward gestures into a history that does
        // not exist.
        view.allowsBackForwardNavigationGestures = false
        // The pane is an article, not an unlabelled group. The GTK reader sets
        // `AccessibleRole::Article` for the same reason: VoiceOver's
        // rotor and its "read from here" both key off the role, and a web view
        // that does not claim one is a region a screen-reader user has no way
        // to enter deliberately. The document's own markup does the rest --
        // headings, paragraphs, and the absence plates' `role="status"`, which
        // is what makes "not downloaded yet" a sentence rather than invisible
        // decoration.
        view.setAccessibilityRole(.group)
        view.setAccessibilityRoleDescription("article")
        view.setAccessibilityLabel(Pane.reader.label)
        coordinator.load(into: view, message: message, remote: remoteImages, original: original)
        return view
    }

    public func updateNSView(_ view: WKWebView, context: Context) {
        context.coordinator.load(into: view, message: message, remote: remoteImages, original: original)
        // After the load, so a page turn that arrives with a new message
        // lands in the document that message produced rather than in the one
        // being replaced.
        context.coordinator.page(view, to: page, token: pageToken)
    }

    /// Holds the handlers and remembers what is on screen.
    ///
    /// `@MainActor` for the same reason the configuration is: a web view, its
    /// scheme handlers and its navigation delegate all live on the main
    /// thread, and a handler called back from anywhere else would be resolving
    /// blobs off it.
    @MainActor
    public final class Coordinator {
        let cid: CidSchemeHandler
        let closed = ClosedSchemeHandler()
        /// The last page turn acted on, so SwiftUI's repeated `updateNSView`
        /// calls do not re-scroll a document somebody has since moved by
        /// hand.
        private var pagedTo: Int?
        let policy: ReaderNavigationPolicy
        private let source: any ReaderSource
        private var showing: Int64?
        private var showingRemote: RemoteImagesFfi = .blocked
        private let onAnswers: ((ReaderNoticeFfi?, String?) -> Void)?
        private var showingOriginal = false
        private let gate = RenderGate()
        private var pending: Task<Void, Never>?
        private let onHeight: ((CGFloat) -> Void)?

        /// Jump to scroll anchor `page`, once per `token`.
        func page(_ view: WKWebView, to page: UInt32, token: Int) {
            guard pagedTo != token else { return }
            pagedTo = token
            // Token zero is the initial state, not a press: acting on it
            // would scroll every message to `pos-0` on open, which is where
            // it already is and one round trip to say so.
            guard token != 0 else { return }
            Task { @MainActor in await ReaderPaging.scroll(view, to: page) }
        }

        init(
            source: any ReaderSource,
            onHeight: ((CGFloat) -> Void)? = nil,
            onAnswers: ((ReaderNoticeFfi?, String?) -> Void)? = nil
        ) {
            self.source = source
            self.onHeight = onHeight
            self.onAnswers = onAnswers
            cid = CidSchemeHandler(source: source)
            let policy = ReaderNavigationPolicy { url in
                // POSTIO-CONSENT: only from a link the user activated inside a
                // message they are reading. The pane does not navigate; the
                // URL goes to whatever the user has chosen as their browser.
                NSWorkspace.shared.open(url)
            }
            self.policy = policy
            guard let onHeight else { return }
            // Measured in the client's own content world, which runs even
            // though the *page* has no JavaScript: `allowsContentJavaScript`
            // is about the sender's markup, and nothing here executes any of
            // it. The measurement is the only thing this asks the document.
            policy.didFinish = { view in
                view.evaluateJavaScript(
                    "document.documentElement.scrollHeight",
                    in: nil,
                    in: .defaultClient
                ) { result in
                    let measured = (try? result.get()) as? Double
                    onHeight(BodyHeight.clamped(measured ?? 0))
                }
            }
        }

        /// Render `message`, or the empty document when there is none.
        ///
        /// The document is built **off the main actor**. It is a SQLite read,
        /// a sanitise and a wrap, and doing that on the cursor's own thread
        /// makes every `j` cost a disk read — `PRODUCT.md` §18 budgets an
        /// interaction at 16 ms, and a large HTML body is not that.
        ///
        /// Which means results can arrive out of order, so each render carries
        /// a token and a stale one is dropped rather than drawn. Drawing it
        /// would put one message's body under another's header, which is the
        /// shape of #70 and the reason `reading.rs` carries the same guard.
        func load(
            into view: WKWebView,
            message: Int64?,
            remote: RemoteImagesFfi,
            original: Bool
        ) {
            guard showing != message || showingRemote != remote || showingOriginal != original
            else { return }
            showing = message
            showingRemote = remote
            showingOriginal = original

            let token = gate.begin()
            pending?.cancel()

            guard let message else {
                view.loadHTMLString("", baseURL: nil)
                cid.message = nil
                return
            }
            // Before the load, so a `postio-cid:` request arriving during it
            // resolves against the right message rather than the previous one.
            cid.message = message

            let source = self.source
            // `view` weakly, like `self`: a reader SwiftUI has let go of is
            // not the render's to keep. Held strongly, a web view outlived its
            // pane for as long as the store took to build a document nobody
            // would see -- a content process kept alive for nothing, and on a
            // busy machine long enough to read as a leak (#1586).
            pending = Task { [weak self, weak view] in
                let answers = await Task.detached {
                    source.readerDocument(message: message, remote: remote, original: original)
                }.value

                guard let self, let view, !Task.isCancelled, self.gate.isCurrent(token) else { return }
                // The one load choke point, and so where a render is counted:
                // a second path to the engine would show up here as two
                // renders for one gesture rather than as a quiet doubling of
                // what `j` costs. The shared counter, the one GTK's reader
                // notes into.
                noteReaderRender()
                view.loadHTMLString(
                    answers.html,
                    baseURL: URL(string: "\(ReaderConfiguration.baseScheme):///")
                )
                // The two by-products of the render (#1589). The notice only
                // comes from a blocked render — an allowed one answers `nil`,
                // and the caller keeps the notice it already has, which its
                // own UI is hiding at that point anyway.
                // Only from a blocked render. The allowed re-render after a
                // grant flip answers no notice, and firing with `nil` would
                // erase the numbers the caller is still holding; its caveat
                // is the same one the blocked render already delivered.
                if remote == .blocked {
                    self.onAnswers?(answers.notice, answers.caveat)
                }
            }
        }
    }
}

/// A reader's web view, which counts its own life (#1586).
///
/// Every web view the reader creates is one of these, so its lifetime **is**
/// the reader's surface count, noted into the counters `postio-gtk` notes
/// into. Counted from the view's own `init` and `deinit` rather than from
/// `makeNSView` and `dismantleNSView`: what costs is the content process
/// behind a web view, and that can go when the view does — which is when ARC
/// gets round to it, not when SwiftUI takes it out of the hierarchy.
///
/// The conversation document's view is one of these as it is: it scrolls its
/// own page. A stacked body's view is `PassingWebView`, which does not.
public class ReaderSurface: WKWebView {
    override public init(frame: CGRect, configuration: WKWebViewConfiguration) {
        super.init(frame: frame, configuration: configuration)
        noteReaderSurfaceCreated()
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("the reader builds its web view in code, with a hardened configuration")
    }

    deinit {
        noteReaderSurfaceReleased()
    }
}

/// A reader web view that lets the scroll wheel through to the pane behind it.
///
/// The single-message pane sizes its body to its whole document —
/// `BodyHeight` is that measurement — so a body never has anything of its own
/// to scroll. `WKWebView` does not know that: it consumes every wheel event
/// over its own frame regardless, and the scroll view around it never sees
/// one.
///
/// Forwarding is unconditionally right here **because** of that sizing: there
/// is no case where this view has scrollable content of its own to keep. If a
/// body ever grows past `BodyHeight.maximum` and starts scrolling internally,
/// this has to become conditional — and that clamp is where to look.
final class PassingWebView: ReaderSurface {
    override func scrollWheel(with event: NSEvent) {
        nextResponder?.scrollWheel(with: event)
    }
}
