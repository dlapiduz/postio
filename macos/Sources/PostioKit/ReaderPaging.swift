import PostioFFI
import WebKit

/// With script off (ADR 0003) WebKit exposes no scroll-by-amount call, so the shared document lays
/// down sixty anchors (`#pos-0`, ...) and a page turn is a same-document jump. The arithmetic is
/// `postio_ui::reader::document`'s.
/// The host's `evaluateJavaScript` still works with `allowsContentJavaScript = false`;
/// `ReaderPagingTests` proves it by reading the scroll position.
public enum ReaderPaging {
    /// The id is Postio's own (`pos-` and a number), built by the boundary, never from a message.
    /// `@MainActor` is required: WebKit kills the process when `evaluateJavaScript` is called off
    /// the main thread, and under the macOS 27 SDK a plain `async` function ran on the cooperative
    /// pool even when awaited from a main-actor task.
    @MainActor
    public static func scroll(_ view: WKWebView, to page: UInt32) async {
        let fragment = readerPageFragment(page: page)
        let script = """
            (() => { const target = document.getElementById("\(fragment)"); \
            if (target) { target.scrollIntoView(); } })()
            """
        await withCheckedContinuation { continuation in
            view.evaluateJavaScript(script) { _, _ in continuation.resume() }
        }
    }
}
