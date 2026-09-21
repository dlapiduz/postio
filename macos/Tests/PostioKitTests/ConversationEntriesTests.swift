import PostioFFI
import Testing
@testable import PostioKit

@Suite struct ConversationEntriesTests {
    private func row(_ id: Int64) -> RowFfi {
        RowFfi(
            id: id,
            thread: 1,
            isThread: false,
            from: "Ada Lovelace",
            fromAddress: "ada@example.com",
            initials: "AL",
            subject: "The gate",
            preview: "Six is fine",
            receivedAt: 1_770_000_000,
            seen: true,
            flagged: false,
            answered: false,
            sendState: nil,
            hasAttachments: false,
            threadCount: 1,
            participants: ""
        )
    }

    private func run(start: UInt32, count: UInt32) -> RunFfi {
        RunFfi(start: start, count: count, summary: "\(count) earlier messages")
    }

    @Test func aMessageIsIdentifiedByTheMessageAndNotItsPosition() {
        // The crash this file exists for. Identity was `"m\(index)"`, so the
        // sixth row of *any* conversation was the same view as far as
        // SwiftUI was concerned.
        let entries = ConversationEntries.of(rows: [row(10), row(20)], runs: [])
        #expect(entries.map(\.id) == ["m10", "m20"])
    }

    @Test func twoConversationsShareNoIdentitiesAtAll() {
        // What actually crashed: a conversation of eight replaced by one of
        // three. SwiftUI kept the child whose id still matched, re-ran it
        // against the shorter `rows`, and `rows[5]` trapped. Positions
        // collide across conversations; message ids do not.
        let long = ConversationEntries.of(rows: (1...8).map { row(Int64($0)) }, runs: [])
        let short = ConversationEntries.of(rows: (100...102).map { row(Int64($0)) }, runs: [])
        #expect(Set(long.map(\.id)).isDisjoint(with: Set(short.map(\.id))))
    }

    @Test func aFoldedRunIsIdentifiedByTheMessageItStartsAt() {
        // Same hazard one row over: `"r\(run.start)"` is a position too, so
        // a run starting at index 3 here and a different run starting at
        // index 3 in the next conversation were one view.
        let rows = [row(10), row(20), row(30), row(40)]
        let entries = ConversationEntries.of(rows: rows, runs: [run(start: 1, count: 2)])
        #expect(entries.map(\.id) == ["m10", "r20", "m40"])
    }

    @Test func aRunStandsInForTheMessagesItCovers() {
        // The walk itself: a folded run consumes its rows, so nothing inside
        // one is also drawn as a message.
        let rows = [row(10), row(20), row(30), row(40), row(50)]
        let entries = ConversationEntries.of(rows: rows, runs: [run(start: 1, count: 3)])
        #expect(entries.count == 3)
        if case let .message(index, id) = entries[2] {
            #expect(index == 4)
            #expect(id == 50)
        } else {
            Issue.record("the last entry should be the message after the run")
        }
    }

    @Test func aRunPastTheEndDoesNotWalkOffTheArray() {
        // `runs` and `rows` are answered separately by the boundary, and a
        // conversation can change between the two reads. Walking off the end
        // is what this whole file is about, so the walk itself must not.
        let entries = ConversationEntries.of(rows: [row(10)], runs: [run(start: 0, count: 9)])
        #expect(entries.count == 1)
        #expect(entries.map(\.id) == ["r10"])
    }

    @Test func noRowsIsNoEntries() {
        #expect(ConversationEntries.of(rows: [], runs: [run(start: 0, count: 2)]).isEmpty)
    }
}
