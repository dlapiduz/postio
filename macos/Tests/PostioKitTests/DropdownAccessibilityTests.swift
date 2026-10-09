import PostioFFI
import Testing

@testable import PostioKit

/// A dropdown row reads as one sentence (specs/010-focus-search T135;
/// design §5): its words, then the folder and date, with the sentence
/// composed in `postio-ui` and highlights never the only signal.
@MainActor
struct DropdownAccessibilityTests {
    @Test func aRowReadsItsWordsThenWhereAndWhen() {
        let row = DropdownModel.Row(
            id: 1, kind: .person,
            title: [RunFfi(text: "Ada ", highlighted: false, style: .plain), RunFfi(text: "Moreno", highlighted: true, style: .plain)],
            detail: [RunFfi(text: "ada@example.com", highlighted: false, style: .plain)],
            folder: nil, right: "yesterday", cap: nil, selectable: true, initials: "AM")
        #expect(DropdownView.accessibleLabel(row) == "Ada Moreno, ada@example.com, yesterday")
    }
}
