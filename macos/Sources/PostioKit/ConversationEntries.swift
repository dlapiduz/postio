import PostioFFI

/// What the conversation stack draws, in order: messages, and dividers
/// standing in for the runs of collapsed ones.
///
/// **Identity is the message, never its position**, and that is the whole
/// reason this is a type rather than four lines inside the view.
///
/// It was `"m\(index)"`. SwiftUI's `ForEach` reuses a child whose id matches,
/// so the sixth row of *any* conversation was the same view as far as the
/// framework was concerned — and selecting a conversation of three after one
/// of eight left a child identified `m5` still being updated against a
/// `rows` array that now had three things in it. `rows[5]` trapped, and the
/// application went away.
///
/// A message id is unique and stable; a position is neither. The same hazard
/// applied to a folded run keyed on `run.start`, so a run is identified by
/// the message it begins at.
public enum ConversationEntries {
    /// One thing the stack draws.
    public enum Entry: Identifiable, Equatable, Sendable {
        /// The message at `index`, drawn expanded or collapsed.
        case message(index: Int, message: Int64)
        /// A divider standing in for the run beginning at `start`.
        case folded(run: RunFfi, message: Int64)

        public var id: String {
            switch self {
            case let .message(_, message): "m\(message)"
            case let .folded(_, message): "r\(message)"
            }
        }
    }

    /// Walk `rows`, folding the runs away.
    ///
    /// Bounds-checked throughout: `rows` and `runs` are answered by two
    /// separate reads and a conversation can change between them, so a run
    /// that claims more rows than are there stops at the end rather than
    /// walking off it. Being careless about exactly that is what this
    /// function was extracted to fix.
    public static func of(rows: [RowFfi], runs: [RunFfi]) -> [Entry] {
        var entries: [Entry] = []
        var index = 0
        while index < rows.count {
            if let run = runs.first(where: { Int($0.start) == index }) {
                entries.append(.folded(run: run, message: rows[index].id))
                // At least one, or a zero-count run would spin here forever.
                index += max(1, Int(run.count))
            } else {
                entries.append(.message(index: index, message: rows[index].id))
                index += 1
            }
        }
        return entries
    }
}
