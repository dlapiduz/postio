import PostioFFI
import Testing

@testable import PostioKit

/// How the keyboard sheet's sections are laid across its columns.
///
/// Across before down, as GTK's sheet read (the classic app's cheat sheet module): a
/// sheet is taken in at a glance, and one tall list in a wide sheet put each
/// key a sheet's width from the name it belongs to.
@Suite struct CheatSheetLayoutTests {
    private func section(_ title: String, _ rows: Int) -> CheatSectionFfi {
        CheatSectionFfi(
            title: title,
            rows: (0..<rows).map {
                CheatRowFfi(id: nil, title: "\(title) \($0)", binding: nil, spoken: "")
            }
        )
    }

    @Test func sectionsKeepTheirOrderAndAllArrive() {
        let sections = [section("A", 5), section("B", 4), section("C", 30), section("D", 12)]
        let columns = CheatSheetLayout.columns(sections, count: 3)
        #expect(columns.flatMap { $0 }.map(\.title) == ["A", "B", "C", "D"])
    }

    @Test func aLongSectionGetsAColumnOfItsOwnRatherThanMakingOneColumnTall() {
        // "Everywhere" and "In the search box" are short and the list's own
        // section is long: grouping by a fixed three per column put all of it
        // under the first two and left the third column nearly empty.
        let sections = [section("A", 5), section("B", 4), section("C", 30), section("D", 12)]
        let columns = CheatSheetLayout.columns(sections, count: 3).map { $0.map(\.title) }
        #expect(columns == [["A", "B"], ["C"], ["D"]])
    }

    @Test func fewerSectionsThanColumnsLeavesNoEmptyColumn() {
        let columns = CheatSheetLayout.columns([section("A", 3)], count: 3)
        #expect(columns.count == 1)
    }

    @Test func nothingToShowIsNoColumns() {
        #expect(CheatSheetLayout.columns([], count: 3).isEmpty)
    }
}
