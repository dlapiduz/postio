import AppKit
import PostioFFI
import PostioKit
import SwiftUI
import WebKit

/// The message window's body (specs/009-focus-macos T069, M5): the engine's
/// treated document in a hardened `WKWebView`.
///
/// - **Hardened** (`ReaderConfiguration.focus`): page script off, a
///   non-persistent store, loaded from a string with no base URL, the
///   document's own CSP, and a content rule list that blocks every load not
///   on Postio's own schemes -- a second wall, so a policy that is present
///   and wrong still fetches nothing (`ReaderEgressTests`).
/// - **App colours**: the web view is transparent and follows the window's
///   appearance; the document's own stylesheet draws it.
/// - **Paper**: light appearance forced on the web view, a white
///   `underPageBackgroundColor`, a radius-6 sheet with a hairline edge, dimmed
///   to `brightness(0.92)` in dark appearance, and zoomed to fit its column
///   no lower than the engine's floor (`PaperFit`). Never inverted, never
///   recoloured.
/// - **Sized to its document**, so the window's one scroll view scrolls the
///   header block and the body together; the web view hands the wheel on.
/// - **The action sentence** is marked in the body, the accent at 8% with a
///   2pt accent underline (SPEC section 5), when the body holds it whole.
///
/// The height, the fit and the mark are Postio's own script, run in the
/// client's content world on the main actor; the sender's script never runs
/// (`MessageBodyTests`).
public struct MessageBodyView: View {
    let message: Int64
    let document: FocusReaderDocumentFfi
    let sentence: String?
    let find: FindInMessage.Request?
    let onFound: (Bool) -> Void
    let resolveCid: (Int64, String) -> InlinePart?
    let resolveFont: (String) -> Data?

    @State private var height: CGFloat = BodyHeight.minimum
    @Environment(\.colorScheme) private var scheme

    /// The sheet's corner.
    public static let sheetRadius: CGFloat = 6
    /// `brightness(0.92)`: in dark appearance the sheet is drawn at 92%, by
    /// laying black at 8% over it.
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
            onHeight: { height = $0 })
            .frame(height: height)
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
}

/// What the body view asks of its document, in the client's content world.
/// On the main actor, where a web view is, and only there.
@MainActor
public enum MessageBodyMeasure {
    /// The document's laid-out size, in CSS pixels.
    public static func size(of view: WKWebView) async throws -> CGSize? {
        let answer = try await view.evaluateJavaScript(
            "[document.documentElement.scrollWidth, document.documentElement.scrollHeight]",
            in: nil, contentWorld: .defaultClient)
        guard let pair = answer as? [Double], pair.count == 2 else { return nil }
        return CGSize(width: pair[0], height: pair[1])
    }

    /// How wide the sender's layout is, in CSS pixels: what the paper fit
    /// divides the column by.
    ///
    /// The page's own width is not it: the reader's body box scrolls
    /// sideways on its own (`overflow-x: auto`), so the page is always the
    /// column's width and what overflows is inside the box. The width is
    /// the page's, plus what the widest scrolling box holds beyond its own.
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

    /// The height once the document's faces have arrived: `postio-font:` is
    /// served asynchronously and `font-display: block` holds the text until
    /// it is, so a height read at the load is the fallback's.
    public static func settledHeight(of view: WKWebView) async throws -> Double? {
        let answer = try await view.callAsyncJavaScript(
            "await document.fonts.ready; return document.documentElement.scrollHeight;",
            arguments: [:], in: nil, contentWorld: .defaultClient)
        return answer as? Double
    }

    /// Mark the first place the body holds `sentence` whole, inside one run
    /// of text, with `fill` behind it and a 2px `line` under it. Whether it
    /// was found: a sentence the sender's markup split across elements is
    /// left unmarked rather than guessed at.
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

    /// The accent as CSS, at `alpha`: the system's accent, whatever the
    /// person chose, read when the body is drawn.
    public static func accent(alpha: Double) -> String {
        let colour = NSColor.controlAccentColor.usingColorSpace(.sRGB) ?? .controlAccentColor
        let channel = { (value: CGFloat) in Int((value * 255).rounded()) }
        return "rgba(\(channel(colour.redComponent)), \(channel(colour.greenComponent)), "
            + "\(channel(colour.blueComponent)), \(alpha))"
    }
}

/// The web view itself.
struct MessageWebView: NSViewRepresentable {
    let message: Int64
    let document: FocusReaderDocumentFfi
    let sentence: String?
    let find: FindInMessage.Request?
    let onFound: (Bool) -> Void
    let resolveCid: (Int64, String) -> InlinePart?
    let resolveFont: (String) -> Data?
    let onHeight: (CGFloat) -> Void

    func makeCoordinator() -> Coordinator {
        Coordinator(resolveCid: resolveCid, resolveFont: resolveFont)
    }

    func makeNSView(context: Context) -> WKWebView {
        let coordinator = context.coordinator
        let configuration = ReaderConfiguration.hardened(
            cidHandler: coordinator.cid, baseHandler: coordinator.closed)
        // Before the view is made: a web view copies its configuration, and
        // a handler set on the copy afterwards is never asked.
        configuration.setURLSchemeHandler(coordinator.font, forURLScheme: ReaderConfiguration.fontScheme)
        let view = MessageBodySurface(frame: .zero, configuration: configuration)
        view.navigationDelegate = coordinator.policy
        view.uiDelegate = coordinator.policy
        view.allowsBackForwardNavigationGestures = false
        view.setAccessibilityRole(.group)
        view.setAccessibilityRoleDescription("article")
        view.setAccessibilityLabel("Message")
        coordinator.show(document, of: message, sentence: sentence, in: view, onHeight: onHeight)
        return view
    }

    func updateNSView(_ view: WKWebView, context: Context) {
        context.coordinator.show(document, of: message, sentence: sentence, in: view, onHeight: onHeight)
        context.coordinator.finder.perform(find, in: view, onFound: onFound)
    }

    @MainActor
    final class Coordinator {
        let cid: CidSchemeHandler
        let font: FontSchemeHandler
        let closed = ClosedSchemeHandler()
        let finder = ReaderFind()
        let policy: ReaderNavigationPolicy
        private var showing: (message: Int64, html: String)?
        private var rulesInstalled = false
        private var task: Task<Void, Never>?

        init(resolveCid: @escaping (Int64, String) -> InlinePart?, resolveFont: @escaping (String) -> Data?) {
            cid = CidSchemeHandler(resolve: resolveCid)
            font = FontSchemeHandler(resolve: resolveFont)
            policy = ReaderNavigationPolicy { url in
                // POSTIO-CONSENT: only from a link the person activated in
                // the message they are reading; the URL goes to their browser.
                NSWorkspace.shared.open(url)
            }
        }

        /// Load `document` unless it is the one on screen.
        func show(
            _ document: FocusReaderDocumentFfi, of message: Int64, sentence: String?,
            in view: WKWebView, onHeight: @escaping (CGFloat) -> Void
        ) {
            guard showing?.message != message || showing?.html != document.html else { return }
            showing = (message, document.html)
            cid.message = message
            let paper = document.treatmentShown == .paper
            // Paper is the sender's page as sent: light, on white, whatever
            // the window's appearance. App colours follow the window.
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
                        onHeight: onHeight)
                }
                noteReaderRender()
                view.loadHTMLString(document.html, baseURL: nil)
            }
        }

        /// The document is in: fit it, mark the sentence, and say how tall.
        private func laidOut(
            _ view: WKWebView, paper: Bool, column: CGFloat, floor: Double, sentence: String?,
            onHeight: @escaping (CGFloat) -> Void
        ) {
            Task { @MainActor [weak view] in
                guard let view else { return }
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
                if let height = try? await MessageBodyMeasure.size(of: view)?.height {
                    onHeight(BodyHeight.clamped(height * zoom))
                }
                if let height = try? await MessageBodyMeasure.settledHeight(of: view) {
                    onHeight(BodyHeight.clamped(height * zoom))
                }
            }
        }
    }
}

/// The body's web view: counted as a reader surface, and handing a
/// vertical wheel to the scroll view around it, since it is sized to its
/// whole document. A paper sheet wider than its column at the floor keeps
/// the sideways wheel, which is the one scroll it has of its own.
final class MessageBodySurface: ReaderSurface {
    override func scrollWheel(with event: NSEvent) {
        if abs(event.scrollingDeltaX) > abs(event.scrollingDeltaY) {
            super.scrollWheel(with: event)
        } else {
            nextResponder?.scrollWheel(with: event)
        }
    }
}
