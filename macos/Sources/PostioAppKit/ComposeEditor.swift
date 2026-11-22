import AppKit
import PostioFFI
import SwiftUI
import WebKit
import PostioKit

/// The composer's editing surface: a `contenteditable` document in the one web view Postio runs
/// script in (#1271). `session.editorScript()` is the engine's script, the same bytes WebKitGTK
/// injects; a Swift copy would be a second dialect that `parse` narrows differently. Only Postio's
/// bridge runs here. Message-borne script never executes: the shell's CSP names no remote origin,
/// so a pasted `<script>` is inert, and the store is non-persistent (ADR 0003).
public struct ComposeEditor: NSViewRepresentable {
    private let session: PostioSession
    private let model: ComposeModel
    private let keyboard: Int
    /// Shift+Tab: the web view keeps it for its own content, so the
    /// composer is told and moves the keyboard to the field above (#2083).
    private let backtab: () -> Void

    public init(
        session: PostioSession, model: ComposeModel, keyboard: Int = 0,
        backtab: @escaping () -> Void = {}
    ) {
        self.session = session
        self.model = model
        self.keyboard = keyboard
        self.backtab = backtab
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(session: session, model: model)
    }

    public func makeNSView(context: Context) -> PastingWebView {
        let coordinator = context.coordinator
        let configuration = WKWebViewConfiguration()
        // Non-persistent: nothing this view does outlives the window.
        configuration.websiteDataStore = .nonPersistent()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true

        let controller = WKUserContentController()
        controller.add(coordinator, name: Bridge.edited)
        controller.add(coordinator, name: Bridge.format)
        controller.addUserScript(
            WKUserScript(
                source: session.editorScript(),
                injectionTime: .atDocumentEnd,
                forMainFrameOnly: true
            )
        )
        configuration.userContentController = controller
        Self.answerSchemes(in: configuration, cid: coordinator.cid)

        let view = PastingWebView(frame: .zero, configuration: configuration)
        view.onBacktab = backtab
        view.onPaste = { [weak coordinator] html in
            coordinator?.paste(html)
        }
        view.onPasteImage = { [weak coordinator] png in
            coordinator?.pasteImage(png)
        }
        view.navigationDelegate = coordinator
        view.setValue(false, forKey: "drawsBackground")
        view.allowsBackForwardNavigationGestures = false
        view.setAccessibilityLabel("Message body")
        coordinator.seed(into: view)
        return view
    }

    public func updateNSView(_ view: PastingWebView, context: Context) {
        context.coordinator.model = model
        view.onBacktab = backtab
        context.coordinator.reseedIfNeeded(view)
        context.coordinator.applyPendingMark(in: view)
        context.coordinator.insertPendingPaste(in: view)
        context.coordinator.insertPendingImage(in: view)
        context.coordinator.takeKeyboard(keyboard, in: view)
    }

    /// Pictures come from the draft's own parts only (#1571): the shell's CSP names `postio-cid:`
    /// as the one image source. The shell's own base is refused like the reader's
    /// (`ClosedSchemeHandler`); without a handler WebKit never started the load.
    static func answerSchemes(in configuration: WKWebViewConfiguration, cid: CidSchemeHandler) {
        configuration.setURLSchemeHandler(cid, forURLScheme: "postio-cid")
        configuration.setURLSchemeHandler(ClosedSchemeHandler(), forURLScheme: Coordinator.baseScheme)
    }

    /// Constants so a rename cannot leave the host listening for a message that is no longer sent.
    enum Bridge {
        static let edited = "postioEdited"
        static let format = "postioFormat"
    }

    @MainActor
    public final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
        private let session: PostioSession
        var model: ComposeModel
        private var seeded: Int64?
        private var appliedMark = 0
        private var insertedImage = 0
        let cid: CidSchemeHandler

        init(session: PostioSession, model: ComposeModel) {
            self.session = session
            self.model = model
            cid = CidSchemeHandler { draft, contentId in
                session.resolveDraftCid(draft: draft, contentId: contentId)
            }
        }

        private var keyboardGranted = 0

        /// On the next turn of the run loop: a surface made in this update has no window yet, and
        /// one made by the Rich switch replaces the plain editor that still has it.
        func takeKeyboard(_ request: Int, in view: WKWebView) {
            guard request != keyboardGranted else { return }
            keyboardGranted = request
            owesKeyboard = true
            DispatchQueue.main.async { [weak self, weak view] in
                guard let self, let view, !view.isLoading else { return }
                self.grantKeyboard(in: view)
            }
        }

        /// A request that arrived while the shell was still loading, for
        /// `didFinish` to grant: a caret set in a page that then loads is lost.
        private var owesKeyboard = false

        private func grantKeyboard(in view: WKWebView) {
            guard owesKeyboard, let window = view.window else { return }
            owesKeyboard = false
            window.makeFirstResponder(view)
            view.evaluateJavaScript(
                "document.body.focus(); "
                    + "const r = document.createRange(); "
                    + "r.setStart(document.body, 0); r.collapse(true); "
                    + "const s = getSelection(); s.removeAllRanges(); s.addRange(r);"
            )
        }

        public func webView(_ view: WKWebView, didFinish navigation: WKNavigation!) {
            grantKeyboard(in: view)
        }

        func seed(into view: WKWebView) {
            seeded = model.id
            cid.message = model.draft.id
            view.loadHTMLString(shell(body: model.bodyHtml ?? ""), baseURL: URL(string: Self.base))
        }

        /// Reloading on every SwiftUI update would move the caret to the start on every keystroke.
        func reseedIfNeeded(_ view: WKWebView) {
            // The draft's store id changes on its first save -- which a
            // picture forces -- and the handler has to follow it, or the
            // picture just inserted resolves against a draft of none.
            cid.message = model.draft.id
            guard seeded != model.id else { return }
            seed(into: view)
        }

        /// Same path as the open panel, so a pasted picture is sent as an inserted one is, not as a
        /// data URL the dialect would drop.
        func pasteImage(_ png: Data) {
            model.insertImage(png, mimeType: "image/png", through: session)
        }

        /// The script is the boundary's (`postio_ui::compose::image_script`), so this host and
        /// WebKitGTK insert the same `<img>`.
        func insertPendingImage(in view: WKWebView) {
            guard let request = model.imageRequest, request.serial != insertedImage else { return }
            insertedImage = request.serial
            view.evaluateJavaScript(request.script)
        }

        /// The clipboard's HTML is narrowed once, in front of the person with a sentence: the store
        /// holds the dialect, so it would be narrowed at the next save anyway with no explanation.
        /// `postio_body` does the narrowing and writes the sentence, so both composers agree.
        func paste(_ html: String) {
            let pasted = session.narrowPaste(html)
            model.tookPaste(pasted)
            pending = pasted.html
        }

        private var pending: String?

        func insertPendingPaste(in view: WKWebView) {
            guard let html = pending else { return }
            pending = nil
            let escaped = html
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "'", with: "\\'")
                .replacingOccurrences(of: "\n", with: "\\n")
            // `insertHTML` at the caret, then the event the host records on
            // -- `execCommand` does not always raise one, and a paste that
            // skipped it would appear on screen and be lost on save.
            view.evaluateJavaScript(
                "document.execCommand('insertHTML', false, '\(escaped)'); "
                    + "document.dispatchEvent(new Event('input'));"
            )
        }

        /// The script is the boundary's (`postio_ui::compose::mark_script`). A command with no
        /// script is not an error: `insert_link` needs an href and takes the other path.
        func applyPendingMark(in view: WKWebView) {
            guard let request = model.markRequest, request.serial != appliedMark else { return }
            appliedMark = request.serial

            let script: String?
            if let href = request.href {
                script = session.linkScript(href)
                if script == nil {
                    model.refuseLink(href)
                    return
                }
            } else {
                script = session.markScript(request.command)
            }
            guard let script else { return }
            view.evaluateJavaScript(script)
        }

        /// A fixed non-`http(s)` base, so edited content is never same-origin with anything real.
        static let base = "\(baseScheme):///"
        static let baseScheme = "postio-editor"

        static let policy = "default-src 'none'; style-src 'unsafe-inline'; img-src postio-cid:"

        private func shell(body: String) -> String {
            """
            <!doctype html><html><head><meta charset="utf-8">
            <meta http-equiv="Content-Security-Policy" content="\(Self.policy)">
            <meta name="color-scheme" content="light dark">
            <style>
              /* The plain body's measures (ComposeView.Metrics): 15pt on a
                 21pt line, its first letter under the field labels' --
                 `-apple-system-body` is 13pt, and the two switched size and
                 place under the writer at every Rich/Plain flip. */
              html { margin: 0; padding: 0; background: transparent; }
              body { margin: 0; padding: 14px 20px; min-height: calc(100vh - 28px);
                     font: 15px/21px -apple-system, sans-serif;
                     color: -apple-system-label; caret-color: -apple-system-label;
                     outline: none; overflow-wrap: break-word; }
              a { color: -apple-system-blue; }
              /* A heading as the reader's app treatment draws one. */
              h1 { font: 600 20px/28px -apple-system, sans-serif; margin: 12px 0 4px; }
              h1:first-child { margin-top: 0; }
              blockquote { margin: 0 0 0 12px; padding-left: 8px;
                           border-left: 2px solid rgba(127,127,127,0.5); }
            </style></head>
            <body contenteditable="true">\(body)</body></html>
            """
        }

        public func userContentController(
            _ controller: WKUserContentController,
            didReceive message: WKScriptMessage
        ) {
            guard let payload = message.body as? String else { return }
            switch message.name {
            case Bridge.edited:
                // The DOM is a working copy; the record is what the boundary makes of it (ADR 0004
                // Q3), narrowed at save so typing costs no parse.
                model.bodyHtml = payload
            case Bridge.format:
                model.caretMarks = Set(payload.split(separator: " ").map(String.init))
            default:
                break
            }
        }

        /// Nothing navigates. A link clicked while editing opens in the
        /// user's own browser, and nothing else is allowed at all.
        public func webView(
            _ view: WKWebView,
            decidePolicyFor action: WKNavigationAction,
            decisionHandler: @escaping (WKNavigationActionPolicy) -> Void
        ) {
            guard action.navigationType == .linkActivated, let url = action.request.url else {
                decisionHandler(action.request.url?.absoluteString.hasPrefix(Self.base) == true
                    ? .allow : .cancel)
                return
            }
            // POSTIO-CONSENT: only from a link the user activated inside the
            // message they are writing. The surface does not navigate.
            NSWorkspace.shared.open(url)
            decisionHandler(.cancel)
        }
    }
}


/// Reading `NSPasteboard` and deciding what ⌘V means is AppKit's job; the decision it feeds,
/// `postio_body::narrow`, is the engine's (`macos/CLAUDE.md`).
public final class PastingWebView: WKWebView {
    public var onPaste: ((String) -> Void)?

    public var onPasteImage: ((Data) -> Void)?

    public var onBacktab: (() -> Void)?

    public override func keyDown(with event: NSEvent) {
        if event.keyCode == 48, event.modifierFlags.contains(.shift), let onBacktab {
            onBacktab()
            return
        }
        super.keyDown(with: event)
    }

    /// `@objc`, not `override`: `WKWebView` implements `paste:` for the responder chain without
    /// exposing it to Swift, so this claims the selector. No `super` call: every branch inserts. A
    /// picture is checked last: a word processor's copy carries a rendered image of the selection
    /// beside its text.
    @objc public func paste(_ sender: Any?) {
        let board = NSPasteboard.general
        if let html = board.string(forType: .html) {
            onPaste?(html)
            return
        }
        if let text = board.string(forType: .string) {
            // Escaped: clipboard text is not markup and must never be read as any.
            onPaste?(escapedParagraph(text))
            return
        }
        if let png = Self.png(on: board) {
            onPasteImage?(png)
        }
    }

    /// Whatever form it was put there in: screenshots and Preview put TIFF, browsers put PNG.
    static func png(on board: NSPasteboard) -> Data? {
        if let png = board.data(forType: .png) { return png }
        guard let tiff = board.data(forType: .tiff),
              let bitmap = NSBitmapImageRep(data: tiff)
        else { return nil }
        return bitmap.representation(using: .png, properties: [:])
    }

    private func escapedParagraph(_ text: String) -> String {
        let escaped = text
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
        return "<p>" + escaped.replacingOccurrences(of: "\n", with: "<br>") + "</p>"
    }
}
