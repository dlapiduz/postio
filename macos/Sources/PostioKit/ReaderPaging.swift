import PostioFFI
import WebKit

/// Turning the page in the reading pane.
///
/// # Why this is a fragment jump and not a scroll call
///
/// The reader is a hardened `WKWebView`: script that arrived in the message
/// is off (ADR 0003), and with it off WebKit exposes no scroll-by-amount
/// call to the host. So the shared document lays down sixty invisible
/// anchors — `#pos-0`, `#pos-1`, … — and a page turn is a same-document jump
/// between them. GTK has done exactly this since it had a reader; the
/// arithmetic and the spelling are `postio_ui::reader::document`'s, so
/// neither frontend can drift from the anchors or from the other.
///
/// The script is the **host's**, which is a different thing from the page's:
/// `allowsContentJavaScript = false` stops the document running its own, and
/// leaves `evaluateJavaScript` working. That is a claim about WebKit rather
/// than about Postio, so `ReaderPagingTests` proves it by reading the scroll
/// position afterwards rather than by checking the call was made.
public enum ReaderPaging {
    /// Jump the view to marker `page`.
    ///
    /// `getElementById` takes a string and never parses a selector, so there
    /// is no selector syntax to escape against — and the id is Postio's own
    /// (`pos-` and a number) rather than anything a message supplied. The
    /// interpolation is still into a string literal, so the id is built by
    /// the boundary rather than spliced here.
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
