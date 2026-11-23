import Foundation
import PostioFFI
import Testing

@testable import PostioKit

@MainActor
struct ThreadBarTests {
    final class Heard: @unchecked Sendable {
        private let lock = NSLock()
        private var events: [UiEvent] = []
        private var task: Task<Void, Never>?

        init(_ session: PostioSession) {
            task = Task.detached { [self] in
                while let event = await session.nextEvent() {
                    lock.withLock { events.append(event) }
                }
            }
        }

        deinit { task?.cancel() }

        func drain() -> [UiEvent] {
            lock.withLock {
                defer { events.removeAll() }
                return events
            }
        }
    }

    @MainActor final class Window {
        let session: PostioSession
        let model: MessageWindowModel
        let heard: Heard
        var rooms: [UInt32] = []
        let menu: ThreadMenuModel
        let outline: ThreadOutlineModel

        init() throws {
            session = try PostioSession.openDemo("threads")
            model = MessageWindowModel(source: session, report: { _, _ in })
            heard = Heard(session)
            menu = ThreadMenuModel(engine: session)
            outline = ThreadOutlineModel(engine: session)
        }

        @MainActor
        func until(_ what: String, _ done: () -> Bool) async throws {
            for _ in 0..<500 {
                for event in heard.drain() { take(event) }
                if done() { return }
                try await Task.sleep(for: .milliseconds(20))
            }
            Issue.record("never saw: \(what)")
        }

        @MainActor
        func take(_ event: UiEvent) {
            menu.apply(event)
            outline.apply(event)
            switch FocusIntents.surface(event) {
            case let .openMessage(message, index, total):
                if model.open(message: message, index: index, total: total, mainWidth: 1440) {
                    session.focusSurfaceOpened(.message)
                    session.focusBarRoom(ThreadBarMetrics.room(window: 760))
                }
            case let .thread(said):
                model.said(said)
            default:
                break
            }
        }

        var ringed: Int? {
            model.bar?.chips.compactMap { chip -> Int? in
                if case let .message(index, _, _, true, _, _) = chip { return Int(index) + 1 }
                return nil
            }.first
        }

        func press(_ keys: String) {
            for key in DemoMode.keys(in: ["POSTIO_DEMO": "threads", "POSTIO_DEMO_KEYS": keys]) {
                if case let .command(id) = session.key(key, in: .list, typing: false) {
                    session.invoke(id)
                }
            }
        }

        @MainActor
        func openHarbor() async throws {
            session.openFocus(.inbox)
            var harbor: UInt32?
            try await until("Harbor's row") {
                let count = session.focusRowCount
                harbor = (0..<count).first {
                    session.focusRow(at: $0)?.subject.lowercased().contains("harbor api draft v3") == true
                }
                return harbor != nil
            }
            guard let harbor else { return }
            session.focusPoint(Int(harbor))
            session.invoke("open_message")
            try await until("the window's bar") { ringed != nil && !model.kicker.isEmpty }
        }
    }

    @Test func theKeyboardWalksTheThreadAndTheRingFollows() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        let opened = try #require(window.ringed, "the window opens with a ringed chip")
        #expect(window.model.bar?.chips.count == 6)

        window.press("[")
        // The bar, kicker and reply line land a turn apart: wait for both.
        try await window.until("`[` rings the message before and shows it") {
            window.ringed == opened - 1 && window.model.kicker.hasPrefix("Message \(opened - 1) of 6")
                && window.model.replyTo != nil
        }
        #expect(window.model.kicker.hasPrefix("Message \(opened - 1) of 6"))
        #expect(window.model.olderBanner != nil, "an older message says so")
        #expect(window.model.replyTo?.hasSuffix("(message 1)") == true)

        window.press("[")
        try await window.until("`[` again") { window.ringed == opened - 2 }

        window.press("}")
        // The ring and the banner land a turn apart: wait for both.
        try await window.until("`}` rings the latest and its banner goes") {
            window.ringed == 6 && window.model.olderBanner == nil
        }

        window.press("{")
        try await window.until("`{` rings the first") { window.ringed == 1 }

        window.press("g u")
        try await window.until("`g u` rings the first unread, the one it opened on") {
            window.ringed == opened
        }
    }

    @Test func aClickOnAChipShowsThatMessageAndMoreOpensTheOutline() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        window.session.focusThreadBarChipClicked(1)
        try await window.until("the click rings message 2") { window.ringed == 2 }

        window.session.focusBarRoom(3)
        var from: UInt32?
        try await window.until("a narrow bar folds chips into More") {
            from = window.model.bar?.chips.compactMap { chip -> UInt32? in
                if case let .more(_, from, _) = chip { return from }
                return nil
            }.first
            return from != nil
        }
        window.session.focusThreadBarMoreClicked(from: try #require(from))
        try await window.until("More opens the outline") { window.outline.view != nil }
    }

    @Test func theMessageStepsInPlaceInTheSameWindow() async throws {
        let window = try Window()
        defer { window.session.shutdown() }
        try await window.openHarbor()
        let first = window.model.shown
        window.press("[")
        try await window.until("another message shown") {
            window.model.shown != nil && window.model.shown != first
        }
        #expect(window.model.isOpen, "the same window, not a new one")
    }

    @Test func aThreadOfOneDrawsNoBar() {
        let window = try? Window()
        defer { window?.session.shutdown() }
        window?.model.said(.bar(ThreadBarFfi(title: "Thread", countLine: "1", chips: [], menuLabel: "Thread", place: "")))
        #expect(window?.model.bar == nil)
    }

    @Test func theBarRoomIsWhatTheWidthLeavesAfterTheLabelAndTheButton() {
        #expect(ThreadBarMetrics.room(window: 760) == 7)
        #expect(ThreadBarMetrics.room(window: 100) == 1, "never none: the message shown draws")
        #expect(ThreadBarMetrics.room(window: 1000) > ThreadBarMetrics.room(window: 800))
    }

    @Test func theBannerAndKickerSitAboveTheSubjectAndReplyToIsTheSenderBlocks() {
        let words = MessageBlocksTests.words()
        let thread = MessageHeaderWords.Thread(
            kicker: "Message 4 of 6 \u{b7} ] newer \u{b7} [ older",
            banner: ThreadBannerFfi(text: "older", action: "Latest", key: "}"),
            replyTo: "Ana Ruiz, Mon 21 Sep (message 1)")
        let action = FocusRowActionFfi(command: "dismiss_marker", label: "Dismiss")
        let view = FocusMessageViewFfi(
            message: 7, subject: "Harbor API draft v3", position: "Message 1 of 3",
            labels: [], addLabel: action, fields: [], date: "", marker: nil, dismiss: action,
            actions: [], more: nil, attachments: [])
        let walking = MessageHeaderWords(view: view, thread: thread, binding: { _ in nil })
        let ids = MessageBlock.blocks(words: walking, document: nil, unsubscribe: .offered).map(\.id)
        #expect(Array(ids.prefix(3)) == ["olderBanner", "kicker", "subject"])
        #expect(MessageBlock.blocks(words: words, document: nil, unsubscribe: .offered).map(\.id)
            .first == "subject", "no walk, no kicker")
    }
}
