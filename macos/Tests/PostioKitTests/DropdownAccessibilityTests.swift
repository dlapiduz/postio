import PostioFFI
import Testing

@testable import PostioKit

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
