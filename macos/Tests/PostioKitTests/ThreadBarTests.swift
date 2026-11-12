import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The message window's thread bar and walk (spec 013 T047, US3).
///
/// The walk is the controller's, so these drive the real one over the
/// threads demo store: the keys go to `session.key`, what comes back is
/// what the window is told, and the window's model is asked which chip is
/// ringed. A fake source could only restate what the test put in it.
@MainActor
struct ThreadBarTests {
    /// Everything the session says, collected as it arrives, so a test can
    /// wait for a fact without owning the event loop.
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

        /// Take what has arrived since the last call.
        func drain() -> [UiEvent] {
            lock.withLock {
                defer { events.removeAll() }
                return events
            }
        }
    }

    /// A window over the threads store, open on Harbor's row, with every
    /// event the controller says fed to its model as the app does.
    @MainActor final class Window {
        let session: PostioSession
        let model: MessageWindowModel
        let heard: Heard
        var rooms: [UInt32] = []

        init() throws {
            session = try PostioSession.openDemo("threads")
            model = MessageWindowModel(source: session, report: { _, _ in })
            heard = Heard(session)
        }

        /// Wait until `done`, feeding the model as events arrive. A poll in
        /// short steps: the wait is for the engine's own turns, not a timer.
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

        /// The 1-based number of the ringed chip.
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

        /// Open Harbor's row in the window.
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
        try await window.until("`[` rings the message before") { window.ringed == opened - 1 }
        #expect(window.model.kicker.hasPrefix("Message \(opened - 1) of 6"))
        #expect(window.model.olderBanner != nil, "an older message says so")
        #expect(window.model.replyTo?.hasSuffix("(message 1)") == true)

        window.press("[")
        try await window.until("`[` again") { window.ringed == opened - 2 }

        window.press("}")
        try await window.until("`}` rings the latest") { window.ringed == 6 }
        #expect(window.model.olderBanner == nil, "the latest has no banner")

        window.press("{")
        try await window.until("`{` rings the first") { window.ringed == 1 }

        window.press("g u")
        try await window.until("`g u` rings the first unread, the one it opened on") {
            window.ringed == opened
        }
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
        window?.model.said(.bar(ThreadBarFfi(title: "Thread", countLine: "1", chips: [], menuLabel: "Thread")))
        #expect(window?.model.bar == nil)
    }

    @Test func theBarRoomIsWhatTheWidthLeavesAfterTheLabelAndTheButton() {
        // 760 - 2*16 edges - 66 label - 8 - 88 button - 8 = 558, 76 a chip.
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
