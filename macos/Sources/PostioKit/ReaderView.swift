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
    private let session: PostioSession
    private let message: Int64?
    private let remoteImages: RemoteImagesFfi

    public init(session: PostioSession, message: Int64?, remoteImages: RemoteImagesFfi = .blocked) {
        self.session = session
        self.message = message
        self.remoteImages = remoteImages
    }

    public func makeCoordinator() -> Coordinator {
        Coordinator(session: session)
    }

    public func makeNSView(context: Context) -> WKWebView {
        let coordinator = context.coordinator
        let configuration = ReaderConfiguration.hardened(
            cidHandler: coordinator.cid,
            baseHandler: coordinator.closed
        )
        let view = WKWebView(frame: .zero, configuration: configuration)
        view.navigationDelegate = coordinator.policy
        view.uiDelegate = coordinator.policy
        view.setValue(false, forKey: "drawsBackground")
        // The pane is Postio's chrome, not a browser: no rubber-banding past
        // the document, and no back-forward gestures into a history that does
        // not exist.
        view.allowsBackForwardNavigationGestures = false
        coordinator.load(into: view, message: message, remote: remoteImages)
        return view
    }

    public func updateNSView(_ view: WKWebView, context: Context) {
        context.coordinator.load(into: view, message: message, remote: remoteImages)
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
        let policy: ReaderNavigationPolicy
        private let session: PostioSession
        private var showing: Int64?
        private var showingRemote: RemoteImagesFfi = .blocked

        init(session: PostioSession) {
            self.session = session
            cid = CidSchemeHandler(session: session)
            policy = ReaderNavigationPolicy { url in
                // POSTIO-CONSENT: only from a link the user activated inside a
                // message they are reading. The pane does not navigate; the
                // URL goes to whatever the user has chosen as their browser.
                NSWorkspace.shared.open(url)
            }
        }

        /// Render `message`, or the empty document when there is none.
        ///
        /// Skips the reload when nothing changed. A `WKWebView` reloaded on
        /// every SwiftUI update would flicker and lose its scroll position on
        /// each unrelated state change, which reads as the application
        /// fighting the user.
        func load(into view: WKWebView, message: Int64?, remote: RemoteImagesFfi) {
            guard showing != message || showingRemote != remote else { return }
            showing = message
            showingRemote = remote

            guard let message else {
                view.loadHTMLString("", baseURL: nil)
                cid.message = nil
                return
            }
            // Before the load, so a `postio-cid:` request arriving during it
            // resolves against the right message rather than the previous one.
            cid.message = message
            let document = session.readerDocument(message: message, remote: remote)
            view.loadHTMLString(document, baseURL: URL(string: "\(ReaderConfiguration.baseScheme):///"))
        }
    }
}
