import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// Thread ▾'s menu, the outline and the new-reply banner (spec 013 US4 and
/// US5, T054 and T066), over the real controller and the threads demo
/// store: the keys go to `session.key`, the models are fed what the engine
/// says, and what a person would see is what is asserted.
@MainActor
struct ThreadMenuOutlineTests {
    typealias Window = ThreadBarTests.Window

    // MARK: the new reply (FR-018)

    @Test func aReplyArrivingWhileReadingOffersItselfAndLeavesTheMessageAlone() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.press("[")
        try await window.until("`[` shows the message before, read whole") {
            window.ringed == 4 && window.model.incoming != nil
        }
        let shown = window.model.shown
        let displayed = window.model.displayed
        let document = window.model.document
        let scroll = window.model.keepsScroll
        let incoming = window.model.incoming

        #expect(window.session.demoArrive(what: "reply"))
        try await window.until("the banner") { window.model.newReply != nil }
        let banner = try #require(window.model.newReply)
        #expect(banner.action == "Read it")
        #expect(banner.key == "}")
        #expect(window.model.shown == shown, "never moved automatically")
        #expect(window.model.displayed == displayed)
        #expect(window.model.document == document, "the body and its place are not redrawn")
        #expect(window.model.keepsScroll == scroll)
        #expect(window.model.incoming == incoming, "no step is in flight because of it")

        window.press("}")
        try await window.until("`}` reads it and the banner goes") {
            window.model.newReply == nil && window.model.shown != shown
        }
    }

    @Test func theNewReplyBannerIsTheFirstBlockOfTheColumn() {
        let words = MessageBlocksTests.words()
        var thread = MessageHeaderWords.Thread(
            kicker: "Message 5 of 7", banner: ThreadBannerFfi(text: "old", action: "Latest", key: "}"),
            replyTo: nil)
        thread.newReply = ThreadBannerFfi(text: "Ana replied", action: "Read it", key: "}")
        let action = FocusRowActionFfi(command: "dismiss_marker", label: "Dismiss")
        let view = FocusMessageViewFfi(
            message: 7, subject: "Harbor", position: "", labels: [], addLabel: action, fields: [],
            date: "", marker: nil, dismiss: action, actions: [], more: nil, attachments: [])
        let walking = MessageHeaderWords(view: view, thread: thread, binding: { _ in nil })
        let ids = MessageBlock.blocks(words: walking, document: nil, unsubscribe: .offered).map(\.id)
        #expect(Array(ids.prefix(3)) == ["newReply", "olderBanner", "kicker"])
        #expect(words.thread.newReply == nil)
    }

    // MARK: the outline's keyboard path

    @Test func theOutlineTakesTypingArrowsAndReturn() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.press("⌥t")
        try await window.until("the outline opens") { window.outline.isOpen }
        let opened = try #require(window.outline.view)
        #expect(opened.rows.count == 6, "every message")
        #expect(opened.people.map(\.count).reduce(0, +) == 6)
        #expect(opened.rows.filter(\.current).count == 1, "the message on screen is ringed")
        let start = opened.cursor

        window.outline.move(by: -1)
        try await window.until("↑ moves the cursor") { window.outline.view?.cursor == max(start, 1) - 1 }
        window.outline.move(by: 1)
        try await window.until("↓ moves it back") { window.outline.view?.cursor == start }

        let word = try #require(
            opened.rows[1].line.split(separator: " ").map(String.init).first { $0.count > 4 }?
                .trimmingCharacters(in: .punctuationCharacters))
        window.outline.typed(word)
        try await window.until("typing narrows the rows") {
            window.outline.view?.filter == word && (window.outline.view?.rows.count ?? 6) < 6
        }
        let narrowed = try #require(window.outline.view)
        let target = narrowed.rows[Int(narrowed.cursor)].message

        window.outline.show()
        try await window.until("Return shows that message") { window.model.shown == target }
    }

    @Test func aPersonChipFiltersAndASecondClickClearsIt() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.press("⌥t")
        try await window.until("the outline opens") { window.outline.isOpen }
        let ben = try #require(window.outline.view?.people.first)
        window.outline.choose(person: ben.name)
        try await window.until("the chip filters") {
            window.outline.view?.person == ben.name && window.outline.view?.rows.count == Int(ben.count)
        }
        window.outline.choose(person: ben.name)
        try await window.until("a second click clears it") {
            window.outline.view?.person == nil && window.outline.view?.rows.count == 6
        }
    }

    // MARK: the menu's keyboard path

    @Test func theMenuTakesArrowsAndReturnAndEscapeClosesIt() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.press("⌥.")
        try await window.until("the menu opens") { window.menu.isOpen }
        let groups = try #require(window.menu.view?.groups)
        #expect(groups.map(\.title) == ["Whole thread", "This message only"])
        #expect(window.menu.highlighted == nil, "no highlight until an arrow")

        window.menu.move(by: 1)
        #expect(window.menu.highlighted == groups[0].entries[0].command, "↓ starts at the top")
        window.menu.move(by: -1)
        #expect(window.menu.highlighted == groups[0].entries[0].command, "and stops there")

        window.menu.confirm()
        try await window.until("Return on Outline opens the outline and puts the menu away") {
            window.outline.isOpen && !window.menu.isOpen
        }
    }

    @Test func escapeClosesTheMenuAndADimmedEntryDoesNothing() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.press("⌥.")
        try await window.until("the menu opens") { window.menu.isOpen }
        let dimmed = window.menu.view?.groups.flatMap(\.entries).first { !$0.enabled }
        if let dimmed {
            window.menu.pick(dimmed.command)
            #expect(window.menu.isOpen, "the first message cannot be split off")
        }
        window.press("⎋")
        try await window.until("Escape closes it") { !window.menu.isOpen }
    }

    @Test func mutingFromTheMenuBadgesTheRow() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.press("⌥.")
        try await window.until("the menu opens") { window.menu.isOpen }
        window.menu.pick("mute_thread")
        try await window.until("the row says muted") {
            let session = window.session
            return (0..<session.focusRowCount).contains {
                let row = session.focusRow(at: $0)
                return row?.subject.lowercased().contains("harbor api draft v3") == true
                    && row?.threadRow?.mutedBadge == "muted"
            }
        }
        let row = try #require((0..<window.session.focusRowCount).compactMap {
            window.session.focusRow(at: $0)
        }.first { $0.subject.lowercased().contains("harbor api draft v3") })
        let model = FocusRowModel(row, binding: { _ in nil })
        #expect(model.mutedBadge == "muted")
    }
}
