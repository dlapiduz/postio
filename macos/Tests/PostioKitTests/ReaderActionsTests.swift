import PostioFFI
import Testing

@testable import PostioKit

/// The bar under an open message.
///
/// #1259 reported the reading pane as unactionable with a mouse. Three of the
/// four verbs had buttons by the time it was worked; `archive` did not, which
/// is the same keyboard-only gap #1221 closed for the list. Which four verbs
/// and in what order is the engine's list — `postio_ui::reader::header` — so
/// this asserts the bar is built from it rather than typed out here again.
@Suite struct ReaderActionsTests {
    private let offered = [
        ReaderActionFfi(command: "reply", title: "Reply", primary: true),
        ReaderActionFfi(command: "reply_all", title: "Reply All", primary: false),
        ReaderActionFfi(command: "forward", title: "Forward", primary: false),
        ReaderActionFfi(command: "archive", title: "Archive", primary: false),
    ]

    private func plan(
        available: @escaping (String) -> Bool = { _ in true },
        bindings: @escaping (String) -> [String] = { _ in [] }
    ) -> [ReaderActionPlan.Item] {
        ReaderActionPlan.items(from: offered, available: available, bindings: bindings)
    }

    @Test func archiveIsReachableWithThePointer() {
        // The reported gap: `a` archived, and nothing on screen said so or
        // offered another way to do it.
        #expect(plan().map(\.command).contains("archive"))
    }

    @Test func theBarIsTheEnginesListInTheEnginesOrder() {
        #expect(plan().map(\.command) == ["reply", "reply_all", "forward", "archive"])
        #expect(plan().map(\.title) == ["Reply", "Reply All", "Forward", "Archive"])
    }

    @Test func exactlyOneVerbIsProminent() {
        #expect(plan().filter(\.prominent).map(\.command) == ["reply"])
    }

    @Test func aButtonNamesTheChordThatDoesTheSameThing() {
        // From the binding in force, not the default: a control that names
        // the wrong key is worse than one that names none — `ToolbarPlan`'s
        // rule, and the same reason.
        // `cmd+shift+a`, not the registry's `mod+shift+a`: the boundary
        // expands `mod` for this platform before a binding ever crosses, and
        // rendering glyphs is all that is left on this side.
        let items = plan(bindings: { $0 == "archive" ? ["cmd+shift+a"] : [] })
        let archive = items.first { $0.command == "archive" }
        #expect(archive?.chord == "⇧⌘A")
        #expect(items.first { $0.command == "reply" }?.chord == nil)
    }

    @Test func aVerbThatCannotRunHereIsDrawnDisabledRatherThanDropped() {
        // Dropped would make the bar change shape as the cursor moves, and
        // "where did Archive go" is a worse question than a greyed button.
        let items = plan(available: { $0 != "archive" })
        #expect(items.map(\.command) == ["reply", "reply_all", "forward", "archive"])
        #expect(items.first { $0.command == "archive" }?.enabled == false)
        #expect(items.first { $0.command == "reply" }?.enabled == true)
    }
}
