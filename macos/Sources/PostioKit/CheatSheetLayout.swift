import PostioFFI

/// How the keyboard sheet's sections are laid across its columns.
///
/// Across before down, as GTK's sheet read (the classic app's cheat sheet module): a
/// sheet is taken in at a glance. The Mac's was one list in a wide sheet,
/// which put each key a sheet's width from the name it belongs to.
///
/// Sections stay in order and are never split -- a heading is the reader's
/// "where am I" -- and the cut points are chosen so the tallest column is as
/// short as it can be. GTK groups three to a column; that rule put a thirty-
/// row section under two short ones and left the last column nearly bare.
public enum CheatSheetLayout {
    /// `sections` in at most `count` columns, no column empty.
    public static func columns(_ sections: [CheatSectionFfi], count: Int) -> [[CheatSectionFfi]] {
        guard !sections.isEmpty, count > 0 else { return [] }
        let heights = sections.map(height)
        // The shortest tallest-column that fits in `count`: no shorter than
        // the tallest single section, no taller than all of them together.
        var low = heights.max() ?? 0
        var high = heights.reduce(0, +)
        while low < high {
            let middle = (low + high) / 2
            if fill(heights, under: middle).count <= count {
                high = middle
            } else {
                low = middle + 1
            }
        }
        return fill(heights, under: low).map { range in Array(sections[range]) }
    }

    /// A section's height in rows: its own rows and its heading, which takes
    /// about two rows of space.
    private static func height(_ section: CheatSectionFfi) -> Int {
        section.rows.count + 2
    }

    /// The runs a greedy fill makes when no column may exceed `limit`.
    private static func fill(_ heights: [Int], under limit: Int) -> [Range<Int>] {
        var runs: [Range<Int>] = []
        var start = 0
        var used = 0
        for (index, height) in heights.enumerated() {
            if used + height > limit, index > start {
                runs.append(start..<index)
                start = index
                used = 0
            }
            used += height
        }
        runs.append(start..<heights.count)
        return runs
    }
}
