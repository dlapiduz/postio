import WebKit

/// Carries out `FindInMessage`'s requests in a reader web view.
///
/// One per reader coordinator. It remembers the last request it acted on,
/// because SwiftUI calls `updateNSView` for every reason there is and a
/// search that re-ran on each would walk the matches on its own.
@MainActor
public final class ReaderFind {
    public init() {}
    private var searched = 0
    private var highlighting = false

    /// Act on `request` if it is new, reporting whether anything matched;
    /// with no request, take away a highlight left by the last one.
    public func perform(_ request: FindInMessage.Request?, in view: WKWebView, onFound: @escaping (Bool) -> Void) {
        guard let request else {
            if highlighting {
                highlighting = false
                // Postio's own script, in its own world: the page's script is
                // off (ADR 0003) and is not what clears this.
                view.evaluateJavaScript(
                    "window.getSelection().removeAllRanges()", in: nil, in: .defaultClient
                )
            }
            return
        }
        guard request.serial != searched else { return }
        searched = request.serial
        highlighting = true
        let configuration = WKFindConfiguration()
        configuration.backwards = request.backwards
        configuration.caseSensitive = false
        configuration.wraps = true
        view.find(request.query, configuration: configuration) { result in
            onFound(result.matchFound)
        }
    }
}
