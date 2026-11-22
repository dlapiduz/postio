import AppKit
import PostioFFI
import SwiftUI
import WebKit
import PostioKit

/// Builds a hardened configuration, hands it a string, refuses navigations. The CSP, fonts,
/// sanitized body and scroll markers arrive from the engine as one document, the same one GTK
/// renders.
public struct ReaderView: NSViewRepresentable {
    private let source: any ReaderSource
    private let message: Int64?
    private let remoteImages: RemoteImagesFfi
    private let reduced: Bool
    private let zoom: Double
    private let find: FindInMessage.Request?
    private let onFound: (Bool) -> Void
    private let onHeight: ((CGFloat) -> Void)?
    private let onAnswers: ((ReaderNoticeFfi?, String?) -> Void)?
    /// A hardened web view has no scroll-by-amount call (message script is off, ADR 0003), so
    /// paging jumps between the document's anchors (`ReaderPaging`). The token, not the page:
    /// paging down at the last anchor leaves the number where it was.
    private let page: UInt32
    private let pageToken: Int

    /// `onHeight` is for a stacked reader: each body is sized to its content, measured from the
    /// laid-out document. `nil` fills what it is given.
    public init(
        source: any ReaderSource,
        message: Int64?,
        remoteImages: RemoteImagesFfi = .blocked,
        reduced: Bool = false,
        zoom: Double = 1,
        find: FindInMessage.Request? = nil,
        onFound: @escaping (Bool) -> Void = { _ in },
        page: UInt32 = 0,
        pageToken: Int = 0,
        onHeight: ((CGFloat) -> Void)? = nil,
        onAnswers: ((ReaderNoticeFfi?, String?) -> Void)? = nil
    ) {
        self.source = source
        self.message = message
        self.remoteImages = remoteImages
        self.reduced = reduced
        self.zoom = zoom
        self.find = find
        self.onFound = onFound
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
        view.allowsBackForwardNavigationGestures = false
        // An article role: VoiceOver's rotor and "read from here" key off the role, as in the GTK
        // reader (`AccessibleRole::Article`).
        view.setAccessibilityRole(.group)
        view.setAccessibilityRoleDescription("article")
        view.setAccessibilityLabel("Message")
        view.pageZoom = zoom
        coordinator.load(into: view, message: message, remote: remoteImages, reduced: reduced)
        return view
    }

    public func updateNSView(_ view: WKWebView, context: Context) {
        // Set only when it moved: assigning re-lays out the page.
        if view.pageZoom != zoom { view.pageZoom = zoom }
        context.coordinator.load(into: view, message: message, remote: remoteImages, reduced: reduced)
        context.coordinator.finder.perform(find, in: view, onFound: onFound)
        // After the load, so a page turn with a new message lands in that message's document.
        context.coordinator.page(view, to: page, token: pageToken)
    }

    /// `@MainActor`: a web view, its scheme handlers and navigation delegate live on the main
    /// thread.
    @MainActor
    public final class Coordinator {
        let cid: CidSchemeHandler
        let closed = ClosedSchemeHandler()
        /// So SwiftUI's repeated `updateNSView` calls do not re-scroll a document somebody has
        /// moved by hand.
        private var pagedTo: Int?
        let finder = ReaderFind()
        let policy: ReaderNavigationPolicy
        private let source: any ReaderSource
        private var showing: Int64?
        private var showingRemote: RemoteImagesFfi = .blocked
        private let onAnswers: ((ReaderNoticeFfi?, String?) -> Void)?
        private var showingReduced = false
        private let gate = RenderGate()
        private var pending: Task<Void, Never>?
        private let onHeight: ((CGFloat) -> Void)?

        func page(_ view: WKWebView, to page: UInt32, token: Int) {
            guard pagedTo != token else { return }
            pagedTo = token
            // Token zero is the initial state, not a press.
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
            // Measured in the client's content world, which runs though the page has no JavaScript:
            // `allowsContentJavaScript` is about the sender's markup.
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

        /// Built off the main actor: a SQLite read, sanitise and wrap on the cursor's thread would
        /// make every `j` a disk read (16 ms budget). Results can arrive out of order, so each
        /// render carries a token and a stale one is dropped (#70).
        func load(
            into view: WKWebView,
            message: Int64?,
            remote: RemoteImagesFfi,
            reduced: Bool
        ) {
            guard showing != message || showingRemote != remote || showingReduced != reduced
            else { return }
            showing = message
            showingRemote = remote
            showingReduced = reduced

            let token = gate.begin()
            pending?.cancel()

            guard let message else {
                view.loadHTMLString("", baseURL: nil)
                cid.message = nil
                return
            }
            // Before the load, so a `postio-cid:` request during it resolves against the right
            // message.
            cid.message = message

            let source = self.source
            // `view` weakly: held strongly, a web view outlived its pane while the store built a
            // document nobody would see (#1586).
            pending = Task { [weak self, weak view] in
                let answers = await Task.detached {
                    source.readerDocument(message: message, remote: remote, reduced: reduced)
                }.value

                guard let self, let view, !Task.isCancelled, self.gate.isCurrent(token) else { return }
                // The one load choke point, where a render is counted: a second path to the engine
                // would show as two renders for one gesture.
                noteReaderRender()
                view.loadHTMLString(
                    answers.html,
                    baseURL: URL(string: "\(ReaderConfiguration.baseScheme):///")
                )
                // Only from a blocked render: an allowed re-render after a grant flip answers no
                // notice, and firing with `nil` would erase the numbers the caller holds.
                if remote == .blocked {
                    self.onAnswers?(answers.notice, answers.caveat)
                }
            }
        }
    }
}

/// Counted from the view's own `init` and `deinit`, not `makeNSView`/`dismantleNSView`: the content
/// process goes when the view does, whenever ARC gets round to it (#1586). A stacked body's
/// `PassingWebView` does not scroll its own page.
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

/// A body sized to its whole document has nothing of its own to scroll, but `WKWebView` consumes
/// every wheel event over its frame. If a body ever scrolls internally (past `BodyHeight.maximum`),
/// forwarding must become conditional.
final class PassingWebView: ReaderSurface {
    override func scrollWheel(with event: NSEvent) {
        nextResponder?.scrollWheel(with: event)
    }
}
