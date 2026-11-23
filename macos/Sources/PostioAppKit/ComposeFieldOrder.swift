import Foundation

/// Explicit rather than left to the key-view loop: the text view treats Shift+Tab as an edit and
/// the web view keeps it for its own content.
enum ComposeField: Hashable, CaseIterable {
    case to, cc, bcc, subject, body

    static func order(showingCopyFields: Bool) -> [ComposeField] {
        showingCopyFields ? [.to, .cc, .bcc, .subject, .body] : [.to, .subject, .body]
    }

    func previous(showingCopyFields: Bool) -> ComposeField? {
        let order = Self.order(showingCopyFields: showingCopyFields)
        guard let index = order.firstIndex(of: self), index > 0 else { return nil }
        return order[index - 1]
    }
}
