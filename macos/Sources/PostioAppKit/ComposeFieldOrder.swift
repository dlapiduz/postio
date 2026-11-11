import Foundation

/// The composer's fields in the order Tab walks them, and so the order
/// Shift+Tab walks back (#2083).
///
/// Explicit rather than left to the key-view loop: the body is a plain
/// text view or a web view, and neither hands Shift+Tab to the field
/// above it -- the text view treats it as an edit, the web view keeps it
/// for its own content. Nothing here decides anything about mail; it is
/// the one list both directions read.
enum ComposeField: Hashable, CaseIterable {
    case to, cc, bcc, subject, body

    /// The fields on screen, top to bottom. Cc and Bcc come and go together
    /// (`copy_fields`).
    static func order(showingCopyFields: Bool) -> [ComposeField] {
        showingCopyFields ? [.to, .cc, .bcc, .subject, .body] : [.to, .subject, .body]
    }

    /// Where Shift+Tab goes from here: the field above, or `nil` at the top,
    /// where the key stays where it is rather than leaving the composer.
    func previous(showingCopyFields: Bool) -> ComposeField? {
        let order = Self.order(showingCopyFields: showingCopyFields)
        guard let index = order.firstIndex(of: self), index > 0 else { return nil }
        return order[index - 1]
    }
}
