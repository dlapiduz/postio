import AppKit
import PostioFFI
import PostioKit
import SwiftUI
import WebKit

/// The message window's body: the engine's treated document in a hardened `WKWebView`
/// (`ReaderConfiguration.focus`): page script off, non-persistent store, no base URL, the
/// document's CSP, and a content rule list blocking every load off Postio's schemes, so a wrong
/// policy still fetches nothing. Sized to its document so the window's one scroll view scrolls
/// header and body together; transparent until it has its height and has painted
/// (`messageBodyDrawn`), since a new web view paints white first. The height, fit and mark are
/// Postio's own script in the client's content world; the sender's never runs.
public struct MessageBodyView: View {
    let message: Int64
    let document: FocusReaderDocumentFfi
    let sentence: String?
    let find: FindInMessage.Request?
    let onFound: (Bool) -> Void
    let resolveCid: (Int64, String) -> InlinePart?
    let resolveFont: (String) -> Data?

    @State private var height: CGFloat = BodyHeight.minimum
    @State private var drawn = false
    @Environment(\.colorScheme) private var scheme
    @Environment(\.messageBodyDrawn) private var sayDrawn

    public static let sheetRadius: CGFloat = 6
    public static let paperDim = 0.08

    public init(
        message: Int64,
        document: FocusReaderDocumentFfi,
        sentence: String?,
        find: FindInMessage.Request?,
        onFound: @escaping (Bool) -> Void,
        resolveCid: @escaping (Int64, String) -> InlinePart?,
        resolveFont: @escaping (String) -> Data?
    ) {
        self.message = message
        self.document = document
        self.sentence = sentence
        self.find = find
        self.onFound = onFound
        self.resolveCid = resolveCid
        self.resolveFont = resolveFont
    }

    private var paper: Bool { document.treatmentShown == .paper }

    public var body: some View {
        let web = MessageWebView(
            message: message, document: document, sentence: sentence, find: find,
            onFound: onFound, resolveCid: resolveCid, resolveFont: resolveFont,
            onHeight: { height = $0 },
            onDrawn: {
                drawn = true
                sayDrawn()
            })
            .frame(height: height)
        Group {
            if paper {
                web
                    .overlay {
                        if scheme == .dark {
                            Rectangle().fill(Color.black.opacity(Self.paperDim)).allowsHitTesting(false)
                        }
                    }
                    .clipShape(RoundedRectangle(cornerRadius: Self.sheetRadius))
                    .overlay(
                        RoundedRectangle(cornerRadius: Self.sheetRadius).strokeBorder(.separator, lineWidth: 1))
            } else {
                web
            }
        }
        .opacity(drawn ? 1 : 0)
    }
}

/// A `<details>` fold opens inside the page with its script off, so the frame kept the closed
/// height and clipped the quote. `toggle` does not bubble, hence the capture listener. Postio's own
/// script, in the client's content world.
@MainActor
final class FoldWatcher {
    static let name = "postioFoldToggled"
    private let onToggle: () -> Void

    init(onToggle: @escaping () -> Void) { self.onToggle = onToggle }

    func install(in configuration: WKWebViewConfiguration) {
        let source = """
            document.addEventListener('toggle', function (event) {
                if (event.target && event.target.tagName === 'DETAILS') {
                    window.webkit.messageHandlers.\(Self.name).postMessage(0);
                }
            }, true);
            """
        configuration.userContentController.addUserScript(
            WKUserScript(
                source: source, injectionTime: .atDocumentStart, forMainFrameOnly: true,
                in: .defaultClient))
        configuration.userContentController.add(
            WeakHandler(self), contentWorld: .defaultClient, name: Self.name)
    }

    private final class WeakHandler: NSObject, WKScriptMessageHandler {
        weak var target: FoldWatcher?
        init(_ target: FoldWatcher) { self.target = target }
        func userContentController(
            _ controller: WKUserContentController, didReceive message: WKScriptMessage
        ) {
            MainActor.assumeIsolated { target?.onToggle() }
        }
    }
}

@MainActor
public enum MessageBodyMeasure {
    public static func size(of view: WKWebView) async throws -> CGSize? {
        let answer = try await view.evaluateJavaScript(
            "[document.documentElement.scrollWidth, document.documentElement.scrollHeight]",
            in: nil, contentWorld: .defaultClient)
        guard let pair = answer as? [Double], pair.count == 2 else { return nil }
        return CGSize(width: pair[0], height: pair[1])
    }

    /// The page's own width is not it: the reader's body box scrolls sideways itself (`overflow-x:
    /// auto`), so the width is the page's plus what the widest scrolling box holds beyond its own.
    public static func layoutWidth(of view: WKWebView) async throws -> Double? {
        let answer = try await view.evaluateJavaScript(
            """
            (function () {
                const page = document.documentElement;
                let beyond = 0;
                for (const box of document.querySelectorAll('.postio-body, .postio-canvas')) {
                    beyond = Math.max(beyond, box.scrollWidth - box.clientWidth);
                }
                return Math.max(page.scrollWidth, page.clientWidth + beyond);
            })()
            """,
            in: nil, contentWorld: .defaultClient)
        return answer as? Double
    }

    /// Less the scroll anchors the shared document lays down for paging: placed at multiples of the
    /// viewport height, they would make any body as tall as the clamp, and a web view that tall
    /// drew nothing.
    static let contentBottom = """
        (function () {
            let bottom = 0;
            for (const child of document.body.children) {
                if (child.tagName === 'A' && /^pos-/.test(child.id)) { continue; }
                const style = getComputedStyle(child);
                bottom = Math.max(bottom, child.getBoundingClientRect().bottom + window.scrollY
                    + parseFloat(style.marginBottom || '0'));
            }
            const body = getComputedStyle(document.body);
            return Math.ceil(bottom + parseFloat(body.paddingBottom || '0')
                + parseFloat(body.marginBottom || '0'));
        })()
        """

    public static func contentHeight(of view: WKWebView) async throws -> Double? {
        let answer = try await view.evaluateJavaScript(contentBottom, in: nil, contentWorld: .defaultClient)
        return answer as? Double
    }

    /// `postio-font:` is served asynchronously and `font-display: block` holds the text until it
    /// is, so a height read at load is the fallback's.
    public static func settledHeight(of view: WKWebView) async throws -> Double? {
        let answer = try await view.callAsyncJavaScript(
            "await document.fonts.ready; return \(contentBottom);",
            arguments: [:], in: nil, contentWorld: .defaultClient)
        return answer as? Double
    }

    /// Bounded, so a view that is not painting (no window) does not hold the body back.
    public static func painted(_ view: WKWebView) async throws {
        _ = try await view.callAsyncJavaScript(
            """
            await Promise.race([
                new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r))),
                new Promise(r => setTimeout(r, 100)),
            ]);
            return true;
            """,
            arguments: [:], in: nil, contentWorld: .defaultClient)
    }

    /// A sentence the sender's markup split across elements is left unmarked rather than guessed
    /// at.
    public static func highlight(
        _ sentence: String, in view: WKWebView, fill: String, line: String
    ) async throws -> Bool {
        let script = """
            if (!sentence) { return false; }
            const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
            let node;
            while ((node = walker.nextNode())) {
                const at = node.data.indexOf(sentence);
                if (at < 0) { continue; }
                const range = document.createRange();
                range.setStart(node, at);
                range.setEnd(node, at + sentence.length);
                const mark = document.createElement('mark');
                mark.className = 'postio-action-sentence';
                mark.style.background = fill;
                mark.style.color = 'inherit';
                mark.style.borderBottom = '2px solid ' + line;
                range.surroundContents(mark);
                return true;
            }
            return false;
            """
        let answer = try await view.callAsyncJavaScript(
            script, arguments: ["sentence": sentence, "fill": fill, "line": line],
            in: nil, contentWorld: .defaultClient)
        return (answer as? Bool) ?? false
    }

    public static func accent(alpha: Double) -> String {
        let colour = NSColor.controlAccentColor.usingColorSpace(.sRGB) ?? .controlAccentColor
        let channel = { (value: CGFloat) in Int((value * 255).rounded()) }
        return "rgba(\(channel(colour.redComponent)), \(channel(colour.greenComponent)), "
            + "\(channel(colour.blueComponent)), \(alpha))"
    }
}

struct MessageWebView: NSViewRepresentable {
    let message: Int64
    let document: FocusReaderDocumentFfi
    let sentence: String?
    let find: FindInMessage.Request?
    let onFound: (Bool) -> Void
    let resolveCid: (Int64, String) -> InlinePart?
    let resolveFont: (String) -> Data?
    let onHeight: (CGFloat) -> Void
    let onDrawn: () -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(resolveCid: resolveCid, resolveFont: resolveFont)
    }

    func makeNSView(context: Context) -> WKWebView {
        let coordinator = context.coordinator
        let configuration = ReaderConfiguration.hardened(
            cidHandler: coordinator.cid, baseHandler: coordinator.closed)
        // Before the view is made: a web view copies its configuration, so a handler set afterwards
        // is never asked.
        configuration.setURLSchemeHandler(coordinator.font, forURLScheme: ReaderConfiguration.fontScheme)
        coordinator.folds.install(in: configuration)
        let view = MessageBodySurface(frame: .zero, configuration: configuration)
        view.onWidth = { [weak coordinator, weak view] _ in
            guard let view else { return }
            coordinator?.resized(view)
        }
        view.navigationDelegate = coordinator.policy
        view.uiDelegate = coordinator.policy
        view.allowsBackForwardNavigationGestures = false
        view.setAccessibilityRole(.group)
        view.setAccessibilityRoleDescription("article")
        view.setAccessibilityLabel("Message")
        coordinator.show(
            document, of: message, sentence: sentence, in: view, onHeight: onHeight, onDrawn: onDrawn)
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        context.coordinator.show(
            document, of: message, sentence: sentence, in: view, onHeight: onHeight, onDrawn: onDrawn)
        context.coordinator.finder.perform(find, in: view, onFound: onFound)
    }

    @MainActor
    final class Coordinator {
        let cid: CidSchemeHandler
        let font: FontSchemeHandler
        let closed = ClosedSchemeHandler()
        let finder = ReaderFind()
        lazy var folds = FoldWatcher { [weak self] in self?.foldToggled() }
        weak var surface: WKWebView?
        let policy: ReaderNavigationPolicy
        private var showing: (message: Int64, html: String)?
        private var rulesInstalled = false
        private var task: Task<Void, Never>?
        private var fit: (paper: Bool, column: CGFloat, floor: Double, onHeight: (CGFloat) -> Void)?
        private var refit: Task<Void, Never>?

        init(resolveCid: @escaping (Int64, String) -> InlinePart?, resolveFont: @escaping (String) -> Data?) {
            cid = CidSchemeHandler(resolve: resolveCid)
            font = FontSchemeHandler(resolve: resolveFont)
            policy = ReaderNavigationPolicy { url in
                // POSTIO-CONSENT: only from a link the person activated in
                // the message they are reading; the URL goes to their browser.
                NSWorkspace.shared.open(url)
            }
        }

        func show(
            _ document: FocusReaderDocumentFfi, of message: Int64, sentence: String?,
            in view: WKWebView, onHeight: @escaping (CGFloat) -> Void, onDrawn: @escaping () -> Void
        ) {
            surface = view
            guard showing?.message != message || showing?.html != document.html else { return }
            showing = (message, document.html)
            fit = nil
            refit?.cancel()
            cid.message = message
            let paper = document.treatmentShown == .paper
            view.appearance = paper ? NSAppearance(named: .aqua) : nil
            view.underPageBackgroundColor = paper ? .white : .clear
            view.setValue(paper, forKey: "drawsBackground")
            view.pageZoom = 1
            let column = CGFloat(document.columnWidth)
            let floor = document.paperFloor
            task?.cancel()
            task = Task { [weak self, weak view] in
                guard let self, let view else { return }
                if !self.rulesInstalled, let rules = try? await ReaderConfiguration.blockingRules() {
                    view.configuration.userContentController.add(rules)
                    self.rulesInstalled = true
                }
                guard !Task.isCancelled else { return }
                self.policy.didFinish = { [weak self] view in
                    self?.laidOut(
                        view, paper: paper, column: column, floor: floor, sentence: sentence,
                        onHeight: onHeight, onDrawn: onDrawn)
                }
                noteReaderRender()
                view.loadHTMLString(document.html, baseURL: nil)
            }
        }

        private func laidOut(
            _ view: WKWebView, paper: Bool, column: CGFloat, floor: Double, sentence: String?,
            onHeight: @escaping (CGFloat) -> Void, onDrawn: @escaping () -> Void
        ) {
            Task { @MainActor [weak self, weak view] in
                guard let view else { return }
                let column = view.frame.width > 0 ? view.frame.width : column
                self?.fit = (paper, column, floor, onHeight)
                var zoom = 1.0
                if paper, let width = try? await MessageBodyMeasure.layoutWidth(of: view) {
                    zoom = PaperFit.zoom(column: column, measured: CGFloat(width), floor: floor)
                    view.pageZoom = zoom
                }
                if let sentence {
                    _ = try? await MessageBodyMeasure.highlight(
                        sentence, in: view, fill: MessageBodyMeasure.accent(alpha: 0.08),
                        line: MessageBodyMeasure.accent(alpha: 1))
                }
                if let height = try? await MessageBodyMeasure.contentHeight(of: view) {
                    onHeight(BodyHeight.clamped(height * zoom))
                }
                try? await MessageBodyMeasure.painted(view)
                onDrawn()
                if let height = try? await MessageBodyMeasure.settledHeight(of: view) {
                    onHeight(BodyHeight.clamped(height * zoom))
                }
            }
        }

        func foldToggled() {
            guard let fit, let view = surface else { return }
            refit?.cancel()
            refit = Task { @MainActor [weak view] in
                guard let view, !Task.isCancelled,
                    let height = try? await MessageBodyMeasure.contentHeight(of: view)
                else { return }
                fit.onHeight(BodyHeight.clamped(height * view.pageZoom))
            }
        }

        func resized(_ view: WKWebView) {
            guard let fit, view.frame.width > 0, view.frame.width != fit.column else { return }
            self.fit = (fit.paper, view.frame.width, fit.floor, fit.onHeight)
            refit?.cancel()
            refit = Task { @MainActor [weak view] in
                guard let view, !Task.isCancelled else { return }
                var zoom = 1.0
                if fit.paper {
                    // Measured unzoomed: at the old zoom a sheet that flows to its column measures
                    // the old column.
                    view.pageZoom = 1
                    if let width = try? await MessageBodyMeasure.layoutWidth(of: view) {
                        guard !Task.isCancelled else { return }
                        zoom = PaperFit.zoom(column: view.frame.width, measured: CGFloat(width), floor: fit.floor)
                        view.pageZoom = zoom
                    }
                }
                guard !Task.isCancelled,
                    let height = try? await MessageBodyMeasure.contentHeight(of: view)
                else { return }
                fit.onHeight(BodyHeight.clamped(height * zoom))
            }
        }
    }
}

/// Hands a vertical wheel to the scroll view around it, since it is sized to its whole document. A
/// paper sheet wider than its column keeps the sideways wheel.
final class MessageBodySurface: ReaderSurface {
    var onWidth: ((CGFloat) -> Void)?

    override func setFrameSize(_ size: NSSize) {
        let was = frame.width
        super.setFrameSize(size)
        if size.width != was, size.width > 0 { onWidth?(size.width) }
    }

    override func scrollWheel(with event: NSEvent) {
        if abs(event.scrollingDeltaX) > abs(event.scrollingDeltaY) {
            super.scrollWheel(with: event)
        } else {
            nextResponder?.scrollWheel(with: event)
        }
    }
}
