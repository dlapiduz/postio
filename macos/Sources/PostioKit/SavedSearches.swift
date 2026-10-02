import Observation
import PostioFFI

/// The saved searches in the sidebar, and which one the keyboard is on.
///
/// A saved search is a `[filters]` entry in `config.toml`. **Swift never
/// parses or writes TOML** (ADR 0031): what crosses is a list of rows and
/// four verbs, and the file is read, patched and written by
/// `postio_ui::saved_search` — the same code `postio-gtk` runs, so a search
/// saved on a Mac and one saved on Linux are the same edit.
///
/// What is local is the cursor and the two questions. `r` and `d` act on the
/// row the sidebar's keyboard is on, and both need a surface a command has no
/// way to present.
@MainActor
@Observable
public final class SavedSearches {
    /// Every saved search, in the order the sidebar draws them.
    public private(set) var rows: [SavedSearchFfi] = []

    /// The key the sidebar's keyboard is on, or `nil` when it is on a folder.
    ///
    /// A key rather than an index: a reorder moves rows under the cursor,
    /// and an index that stayed put would end up on whichever row slid into
    /// that position.
    public private(set) var cursor: String?

    /// What a command has asked the sidebar to put on screen.
    public enum Wish: Equatable, Sendable {
        /// Ask for a new name for this row, starting from the one it has.
        case rename(key: String, from: String)
        /// Ask whether this row really goes.
        case confirmDelete(key: String, name: String)
    }

    public private(set) var wish: Wish?
    public private(set) var wishToken = 0

    public init() {}

    /// The row the keyboard is on, out of the ones there are.
    ///
    /// `nil` when the cursor is on a folder, and when it names a row that is
    /// no longer in the file — `config.toml` is hand-edited and watched, so a
    /// row can go while a window is open. Doing nothing is the truthful
    /// outcome; acting on the nearest row would edit a search nobody aimed
    /// at.
    public var focused: SavedSearchFfi? {
        guard let cursor else { return nil }
        return rows.first { $0.key == cursor }
    }

    /// Put the keyboard on a saved search, or take it off them.
    public func put(cursor: String?) {
        self.cursor = cursor
    }

    /// Ask the sidebar for `wish`.
    public func ask(_ wish: Wish) {
        self.wish = wish
        wishToken += 1
    }

    /// Read the file.
    public func load(from path: String) {
        rows = savedSearches(path: path)
    }

    /// Take the result of one of the four verbs.
    ///
    /// The cursor follows the row that changed, which is what makes a
    /// reorder repeatable: after `⇧↑` the keyboard belongs on the row that
    /// moved, not on the position it vacated, or a second press would walk a
    /// different row. `changed` being `nil` means nothing happened — a row
    /// already at the end it was moving toward — and the cursor stays where
    /// it is rather than flashing a change that did not occur.
    public func apply(_ edit: SavedSearchEditFfi) {
        rows = edit.searches
        if let changed = edit.changed {
            cursor = changed
        }
    }
}
