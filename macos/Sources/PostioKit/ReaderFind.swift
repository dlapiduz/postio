import WebKit

/// Remembers the last request it acted on: SwiftUI calls `updateNSView` for every reason and a
/// search re-run on each would walk the matches.
@MainActor
public final class ReaderFind {
    public init() {}
    private var searched = 0
    private var highlighting = false

    public func perform(_ request: FindInMessage.Request?, in view: WKWebView, onFound: @escaping (Bool) -> Void) {
        guard let request else {
            if highlighting {
                highlighting = false
                // Postio's own script, in its own world: the page's script is off (ADR 0003).
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
        configuration.wraps = !request.bounded
        view.find(request.query, configuration: configuration) { result in
            onFound(result.matchFound)
        }
    }
}
