import AppKit
import PostioFFI
import SwiftUI
import WebKit

/// The composer's editing surface: a `contenteditable` document in the one
/// web view Postio runs script in (#1271).
///
/// The body was a `TextEditor`, so the format bar was decoration and `rich`
/// was a claim the window could not keep. A rich mail composer needs a
/// document, and on both of Postio's frontends the document is a web view
/// over `postio_body`'s dialect — GTK's `editor.rs` is the same shape, and
/// deliberately so.
///
/// # The script is the engine's, not this file's
///
/// `session.editorScript()` is `postio_ui::compose::EDITOR_SCRIPT`, the same
/// bytes WebKitGTK injects. It pins `defaultParagraphSeparator` to `p` and
/// `styleWithCSS` off, which is what makes the surface emit `<p>` paragraphs
/// and element-form `<strong>`/`<em>` rather than `<div>`s full of
/// `<span style>`. A second copy written in Swift would be a second dialect:
/// `parse` would narrow it differently, and the two composers would disagree
/// about what the same keystrokes wrote while both still round-tripped
/// through a `Document`. `window.webkit.messageHandlers` is the same API on
/// both engines, which is why one file serves both.
///
/// # Script runs here and nowhere else
///
/// ADR 0003's licence, at the place it applies: this view runs *Postio's*
/// bridge over a document being edited. Message-borne script never executes
/// anywhere — the reader keeps JavaScript off entirely, and here the shell's
/// CSP names no remote origin, so a `<script>` that arrives in a paste is
/// inert markup. Network is closed the same way the reader closes it: a
/// non-persistent data store, no navigation, and a CSP with no remote
/// source.
public struct ComposeEditor: NSViewRepresentable {
    private let session: PostioSession
    private let model: ComposeModel

    public init(session: PostioSession, model: ComposeModel) {
        self.session = session
        self.model = model
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(session: session, model: model)
    }

    public func makeNSView(context: Context) -> PastingWebView {
        let coordinator = context.coordinator
        let configuration = WKWebViewConfiguration()
        // Nothing this view does should outlive the window: a composer is not
        // a browser profile.
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
        // Pictures in the body, from the draft's own parts and nowhere else
        // (#1571). The shell's CSP has always named `postio-cid:` as the one
        // image source it allows; nothing answered it, so a draft reopened
        // with a picture in it drew a broken one.
        configuration.setURLSchemeHandler(coordinator.cid, forURLScheme: "postio-cid")

        let view = PastingWebView(frame: .zero, configuration: configuration)
        view.onPaste = { [weak coordinator] html in
            coordinator?.paste(html)
        }
        view.onPasteImage = { [weak coordinator] png in
            coordinator?.pasteImage(png)
        }
        view.navigationDelegate = coordinator
        view.setValue(false, forKey: "drawsBackground")
        // Chrome, not a browser: no rubber-banding past the document and no
        // gestures into a history that does not exist.
        view.allowsBackForwardNavigationGestures = false
        view.setAccessibilityLabel("Message body")
        coordinator.seed(into: view)
        return view
    }

    public func updateNSView(_ view: PastingWebView, context: Context) {
        context.coordinator.model = model
        context.coordinator.reseedIfNeeded(view)
        context.coordinator.applyPendingMark(in: view)
        context.coordinator.insertPendingPaste(in: view)
        context.coordinator.insertPendingImage(in: view)
    }

    /// The two channels the bridge reports on, named as the script names
    /// them. Constants rather than literals so a rename cannot leave the
    /// host listening for a message that is no longer sent — which would be
    /// a composer that silently stopped saving.
    enum Bridge {
        static let edited = "postioEdited"
        static let format = "postioFormat"
    }

    /// Holds the channels and remembers what has been loaded.
    @MainActor
    public final class Coordinator: NSObject, WKScriptMessageHandler, WKNavigationDelegate {
        private let session: PostioSession
        var model: ComposeModel
        /// The document this surface was seeded with, so an edit round trip
        /// does not reload it under the caret.
        private var seeded: Int64?
        /// The last format press this surface has run, so a SwiftUI update
        /// for any other reason does not re-apply it.
        private var appliedMark = 0
        /// The last picture this surface has drawn, for the same reason.
        private var insertedImage = 0
        /// Answers `postio-cid:` from the draft's own parts.
        let cid: CidSchemeHandler

        init(session: PostioSession, model: ComposeModel) {
            self.session = session
            self.model = model
            cid = CidSchemeHandler { draft, contentId in
                session.resolveDraftCid(draft: draft, contentId: contentId)
            }
        }

        /// Load the editing shell with whatever the draft already holds.
        func seed(into view: WKWebView) {
            seeded = model.id
            cid.message = model.draft.id
            view.loadHTMLString(shell(body: model.bodyHtml ?? ""), baseURL: URL(string: Self.base))
        }

        /// Reload only when the window has been pointed at a different
        /// draft. Reloading on every SwiftUI update would move the caret to
        /// the start of the document on every keystroke.
        func reseedIfNeeded(_ view: WKWebView) {
            // The draft's store id changes on its first save -- which a
            // picture forces -- and the handler has to follow it, or the
            // picture just inserted resolves against a draft of none.
            cid.message = model.draft.id
            guard seeded != model.id else { return }
            seed(into: view)
        }

        /// A picture pasted from the clipboard, as PNG bytes.
        ///
        /// The same path as the open panel: the boundary stores the part and
        /// the surface draws it, so a pasted picture is sent the way an
        /// inserted one is rather than as a data URL the dialect would drop.
        func pasteImage(_ png: Data) {
            model.insertImage(png, mimeType: "image/png", through: session)
        }

        /// Draw the picture the model is holding, if it is a new one.
        ///
        /// The script is the boundary's -- `postio_ui::compose::image_script`
        /// -- so this host and WebKitGTK insert the same `<img>`.
        func insertPendingImage(in view: WKWebView) {
            guard let request = model.imageRequest, request.serial != insertedImage else { return }
            insertedImage = request.serial
            view.evaluateJavaScript(request.script)
        }

        /// Take a paste: narrow it, insert it, and say what that cost.
        ///
        /// The clipboard's HTML is whatever the source application felt like
        /// producing -- a browser's tables, a word processor's inline styles.
        /// Letting the surface keep it would not save it: the store holds the
        /// dialect, so it would be narrowed at the next save anyway, and the
        /// person would watch their table become four lines of text with
        /// nothing on screen explaining why. Narrowing here means it is
        /// narrowed *once*, in front of them, with a sentence.
        ///
        /// `postio_body` does the narrowing and writes the sentence, so both
        /// composers say the same thing about the same paste.
        func paste(_ html: String) {
            let pasted = session.narrowPaste(html)
            model.tookPaste(pasted)
            pending = pasted.html
        }

        /// A narrowed paste waiting to be inserted at the caret.
        private var pending: String?

        /// Insert whatever the last paste narrowed to.
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

        /// Run the format press the model is holding, if it is a new one.
        ///
        /// The script comes from the boundary --
        /// `postio_ui::compose::mark_script` -- rather than being written
        /// here, so this host and WebKitGTK do the same thing about `bold`.
        /// A command with no script is not an error: the bar carries
        /// `insert_link`, which needs an href and takes the other path.
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

        /// A fixed, non-`http(s)` base, so edited content is never
        /// same-origin with anything real — the reader's reasoning, applied
        /// to the one view that does run script.
        static let base = "postio-editor:///"

        /// The CSP: no remote origin can be named, and the only styles are
        /// the shell's own.
        static let policy = "default-src 'none'; style-src 'unsafe-inline'; img-src postio-cid:"

        private func shell(body: String) -> String {
            """
            <!doctype html><html><head><meta charset="utf-8">
            <meta http-equiv="Content-Security-Policy" content="\(Self.policy)">
            <style>
              html, body { margin: 0; padding: 12px; background: transparent; }
              body { font: -apple-system-body; color: -apple-system-label;
                     outline: none; }
              blockquote { margin: 0 0 0 12px; padding-left: 8px;
                           border-left: 2px solid rgba(127,127,127,0.5); }
            </style></head>
            <body contenteditable="true">\(body)</body></html>
            """
        }

        /// The bridge reporting an edit, or the caret's formatting.
        public func userContentController(
            _ controller: WKUserContentController,
            didReceive message: WKScriptMessage
        ) {
            guard let payload = message.body as? String else { return }
            switch message.name {
            case Bridge.edited:
                // The DOM is a working copy; the record is what the boundary
                // makes of it (ADR 0004 Q3). Kept as reported here and
                // narrowed at save, so typing costs no parse.
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
                // The initial `loadHTMLString` is the only load that happens.
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


/// A `WKWebView` that hands its pastes to the host first.
///
/// The interception is here rather than in the shared bridge script because
/// it is genuinely platform work: reading `NSPasteboard` and deciding what
/// `⌘V` means is AppKit's job, and the *decision* it feeds --
/// `postio_body::narrow` -- is the engine's and is shared. That split is
/// `macos/CLAUDE.md`'s rule: Swift gets views and platform observation,
/// anything that decides something stays in Rust.
public final class PastingWebView: WKWebView {
    /// Called with the clipboard's HTML, or its plain text escaped into a
    /// paragraph when it carries no HTML at all.
    public var onPaste: ((String) -> Void)?

    /// Called with a picture from the clipboard, as PNG bytes, when it
    /// carries neither HTML nor text (#1571).
    public var onPasteImage: ((Data) -> Void)?

    /// `@objc` and not `override`: `WKWebView` implements `paste:` for the
    /// responder chain but does not expose it to Swift, so this is the same
    /// selector claimed rather than a method overridden. AppKit dispatches
    /// `⌘V` dynamically and finds this one.
    ///
    /// There is no `super` call, and nothing is lost by that: every branch
    /// below ends in an insertion. A picture is checked **last**, after the
    /// text: a word processor's copy carries a rendered image of the
    /// selection beside its text, and pasting that would put a picture of
    /// somebody's paragraph where they meant the paragraph.
    @objc public func paste(_ sender: Any?) {
        let board = NSPasteboard.general
        if let html = board.string(forType: .html) {
            onPaste?(html)
            return
        }
        if let text = board.string(forType: .string) {
            // Plain text still goes through the narrowing path, so that one
            // code path inserts and one sentence explains. Escaped first:
            // clipboard text is not markup and must never be read as any.
            onPaste?(escapedParagraph(text))
            return
        }
        if let png = Self.png(on: board) {
            onPasteImage?(png)
        }
    }

    /// The clipboard's picture as PNG bytes, whatever form it was put there
    /// in — screenshots and Preview put TIFF, browsers put PNG.
    static func png(on board: NSPasteboard) -> Data? {
        if let png = board.data(forType: .png) { return png }
        guard let tiff = board.data(forType: .tiff),
              let bitmap = NSBitmapImageRep(data: tiff)
        else { return nil }
        return bitmap.representation(using: .png, properties: [:])
    }

    /// Clipboard text as a paragraph, with nothing in it that could be read
    /// as markup.
    private func escapedParagraph(_ text: String) -> String {
        let escaped = text
            .replacingOccurrences(of: "&", with: "&amp;")
            .replacingOccurrences(of: "<", with: "&lt;")
            .replacingOccurrences(of: ">", with: "&gt;")
        return "<p>" + escaped.replacingOccurrences(of: "\n", with: "<br>") + "</p>"
    }
}
