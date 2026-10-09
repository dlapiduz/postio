import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// What search says to a screen reader (specs/010-focus-search T135;
/// design §5): the field is a combobox over a listbox, the results are a
/// table, and every word comes from `postio-ui` rather than a literal here.
@MainActor
@Suite struct SearchAccessibilityTests {
    static let words = focusSearchWords()

    final class BarEngine: CommandBarEngine {
        func focusBarTyped(_ text: String) {}
        func focusBarRun(_ token: UInt64) {}
        func focusBarTab() -> Bool { true }
        func invoke(_ id: String) {}
        func binding(for command: String) -> String? { nil }
        func focusSearchForget(_ token: UInt64) {}
        func focusSearchShowAll() {}
        func focusSearchHighlighted(_ token: UInt64) {}
        func focusSearchExclude(_ token: UInt64) {}
    }

    @Test func theToolbarFieldIsAComboboxThatSaysWhetherItsListIsOpen() {
        let box = ToolbarSearchBox()
        #expect(box.field.accessibilityRole() == .comboBox)
        #expect(box.field.accessibilityLabel() == Self.words.queryLabel)
        #expect(box.field.isAccessibilityExpanded() == false)
        box.open(true, windowWidth: 1200)
        #expect(box.field.isAccessibilityExpanded() == true)
        box.open(false, windowWidth: 1200)
        #expect(box.field.isAccessibilityExpanded() == false)
    }

    @Test func theResultsQueryIsAComboboxToo() {
        let field = ChipQueryField(frame: NSRect(x: 0, y: 0, width: 400, height: 34))
        #expect(field.accessibilityRole() == .comboBox)
        #expect(field.accessibilityLabel() == Self.words.queryLabel)
    }

    @Test func theSuggestionsPanelIsAListNamedByRust() {
        let panel = CommandBarPanel(model: CommandBarModel(engine: BarEngine()), saveCap: { nil })
        #expect(panel.listAccessibilityRole == .list)
        #expect(panel.listAccessibilityLabel == Self.words.suggestionsLabel)
    }

    @Test func theResultsAreATableAndTheFilesAGridNamedByRust() {
        let results = ResultsTable(model: ResultsModel(engine: FilesGridTests.Engine()))
        #expect(results.tableView.accessibilityLabel() == Self.words.resultsLabel)
        let files = FilesGrid(model: ResultsModel(engine: FilesGridTests.Engine()))
        #expect(files.collectionView.accessibilityLabel() == Self.words.filesLabel)
    }
}
