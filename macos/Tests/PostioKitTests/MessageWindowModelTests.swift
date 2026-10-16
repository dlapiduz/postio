import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The message window's state, as the controller's intents leave it
/// (specs/009-focus-macos T070).
///
/// The controller decides which message is open, what `j` reaches, and what
/// Back closes first; this model only holds the answers and fetches what to
/// draw. These pin the few things the Mac does itself: a step replaces the
/// content rather than reopening the window, `[`/`]` show the message the
/// engine named, `O` asks for the other treatment without reloading the
/// chrome, `v` puts the source in place, and More and find are reported so
/// the controller's Back can close them first.
@MainActor
struct MessageWindowModelTests {
    /// Answers from canned records, and remembers what it was asked.
    final class Source: MessageWindowSource, @unchecked Sendable {
        private let lock = NSLock()
        private var _views: [(Int64, UInt32, UInt32)] = []
        private var _documents: [(Int64, TreatmentFfi?, Int32)] = []
        var earlier: Int64?
        var later: Int64?
        var shown: TreatmentFfi = .paper
        var raw: Result<Data, Error> = .success(Data("Received: by example.com\r\n".utf8))
        /// When set, every document read waits for it: a body still being
        /// read while its chrome is in.
        var hold: DispatchSemaphore?

        var views: [(Int64, UInt32, UInt32)] { lock.withLock { _views } }
        var documents: [(Int64, TreatmentFfi?, Int32)] { lock.withLock { _documents } }

        func focusMessageView(message: Int64, index: UInt32, total: UInt32) -> FocusMessageViewFfi {
            lock.withLock { _views.append((message, index, total)) }
            return MessageWindowModelTests.view(message, earlier: earlier, later: later)
        }

        func focusReaderDocument(
            message: Int64, remote: RemoteImagesFfi, chosen: TreatmentFfi?, mainWidth: Int32
        ) -> FocusReaderDocumentFfi {
            lock.withLock { _documents.append((message, chosen, mainWidth)) }
            if let hold { hold.wait() }
            return MessageWindowModelTests.document(shown: chosen ?? shown, of: message)
        }

        func rawSource(_ message: Int64) throws -> Data { try raw.get() }

        private var _activated: [Int64] = []
        /// What `activateUnsubscribe` answers: `nil` when it was recorded.
        var refusal: String?
        /// Every message an unsubscribe was recorded for, in order.
        var activated: [Int64] { lock.withLock { _activated } }

        func activateUnsubscribe(_ message: Int64) -> String? {
            lock.withLock { _activated.append(message) }
            return refusal
        }
    }

    nonisolated static func view(_ message: Int64, earlier: Int64? = nil, later: Int64? = nil) -> FocusMessageViewFfi {
        let action = FocusRowActionFfi(command: "dismiss_marker", label: "Dismiss")
        return FocusMessageViewFfi(
            message: message, subject: "Harbor API draft v3", position: "Message 1 of 3",
            thread: nil, earlier: earlier, later: later, labels: [], addLabel: action,
            fields: [], date: "Today, 15:22", marker: nil, dismiss: action, actions: [],
            more: nil, attachments: [])
    }

    nonisolated static func document(shown: TreatmentFfi, of message: Int64 = 0) -> FocusReaderDocumentFfi {
        FocusReaderDocumentFfi(
            html: "<p>\(message)</p>", notice: nil, caveat: nil, unsubscribe: nil,
            treatmentShown: shown,
            treatmentClassified: .paper, renderMode: nil, sender: "news@example.com",
            senderChoice: nil, windowWidth: 720, columnWidth: shown == .paper ? 640 : 560,
            foldsIntoMore: false, paperFloor: 0.85)
    }

    /// The model, and every reader state it reported, in order.
    final class Reports { var said: [(Bool, Bool)] = [] }

    static func model(_ source: Source) -> (MessageWindowModel, Reports) {
        let reports = Reports()
        let model = MessageWindowModel(source: source) { more, finding in
            reports.said.append((more, finding))
        }
        return (model, reports)
    }

    @Test func returnOpensTheRowsMessageAndSaysTheWindowIsNew() async {
        let source = Source()
        let (model, _) = Self.model(source)
        #expect(!model.isOpen)
        #expect(model.open(message: 7, index: 2, total: 9, mainWidth: 1440), "a fresh open")
        await model.settled()
        #expect(model.isOpen)
        #expect(model.view?.message == 7)
        #expect(model.document?.windowWidth == 720)
        #expect(source.views.map(\.0) == [7] && source.views.first?.1 == 2)
        #expect(source.documents.first?.2 == 1440, "sized from the main window's width")
    }

    @Test func jReplacesTheContentWithoutReopening() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(!model.open(message: 8, index: 3, total: 9, mainWidth: 1024), "the same window")
        await model.settled()
        #expect(model.view?.message == 8)
        #expect(model.place?.index == 3)
        #expect(
            source.documents.last?.2 == 1440,
            "the window keeps the width it opened with, so j never resizes it")
    }

    /// Wait, a little at a time, for `done`.
    static func until(_ done: () -> Bool) async throws {
        for _ in 0..<400 where !done() {
            try await Task.sleep(for: .milliseconds(5))
        }
    }

    @Test func aStepKeepsTheMessageWholeUntilTheNextBodyIsRead() async throws {
        // A header over the last message's body, or over none, is what the
        // window flickered through on every `j`.
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        let hold = DispatchSemaphore(value: 0)
        source.hold = hold
        model.open(message: 8, index: 3, total: 9, mainWidth: 1440)
        try await Self.until { source.views.contains { $0.0 == 8 } }
        try await Task.sleep(for: .milliseconds(50))
        #expect(model.view?.message == 7, "no header over a body not yet read")
        #expect(model.document?.html == "<p>7</p>")
        hold.signal()
        source.hold = nil
        try await Self.until { model.incoming != nil }
        #expect(model.incoming?.message == 8, "read whole, and drawn out of sight")
        #expect(model.view?.message == 7, "nor until that body is drawn")
        model.bodyDrawn(7)
        #expect(model.displayed == 7, "only the incoming body puts itself on screen")
        model.bodyDrawn(8)
        #expect(model.displayed == 8 && model.incoming == nil)
        #expect(model.view?.message == 8 && model.document?.html == "<p>8</p>")
        #expect(!model.keepsScroll, "a new message is read from its top")
    }

    @Test func aBodyThatNeverSaysItIsDrawnIsShownAnyway() async throws {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(model.displayed == 7, "a fresh window has nothing to keep, so waits for nothing")
        let started = ContinuousClock.now
        model.open(message: 8, index: 3, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(model.displayed == 8)
        #expect(ContinuousClock.now - started >= MessageWindowModel.drawPatience)
    }

    @Test func theSameMessageAgainReadsNothing() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(source.views.count == 1)
    }

    @Test func aStepThatOvertakesALoadIsTheOneDrawn() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        model.open(message: 8, index: 3, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(model.view?.message == 8, "a stale answer is dropped, not drawn")
    }

    @Test func shiftOAsksForTheOtherTreatmentAndKeepsTheChrome() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(model.document?.treatmentShown == .paper)
        model.apply(.switchTreatment)
        await model.settled()
        #expect(source.documents.last?.1 == .appColours)
        #expect(model.document?.treatmentShown == .appColours)
        #expect(source.views.count == 1, "the chrome is the same message's")
        #expect(model.keepsScroll, "the switch keeps the place being read")
        model.apply(.switchTreatment)
        await model.settled()
        #expect(source.documents.last?.1 == .paper)
    }

    @Test func aBodyThatLandsWhileOpenIsReadAgainInPlace() async {
        // #1788: the window drew "Downloading this message" and never
        // looked again when the words came.
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.switchTreatment)
        await model.settled()
        let documents = source.documents.count
        model.bodyLanded(8)
        await model.settled()
        #expect(source.documents.count == documents, "a body nobody is reading")
        model.bodyLanded(7)
        await model.settled()
        #expect(source.documents.count == documents + 1)
        #expect(source.documents.last?.0 == 7)
        #expect(source.documents.last?.1 == .appColours, "the treatment chosen stays")
        #expect(source.views.last?.0 == 7, "its chips too: a fetch replaces them")
        #expect(model.keepsScroll, "the place being read stays")
        #expect(model.place?.message == 7 && model.shown == 7)
    }

    @Test func aBodyThatLandsUnderTheSourceLeavesTheSource() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.viewSource)
        await model.settled()
        let documents = source.documents.count
        model.bodyLanded(7)
        await model.settled()
        #expect(model.showingSource)
        #expect(source.documents.count == documents)
    }

    @Test func aBracketShowsTheMessageTheEngineNamedInPlace() async {
        let source = Source()
        source.earlier = 5
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.stepThread(by: -1))
        await model.settled()
        #expect(model.shown == 5)
        #expect(model.place?.message == 7, "the row is still the one the list is on")
        #expect(source.views.last.map { [$0.0, Int64($0.1), Int64($0.2)] } == [5, 2, 9])
    }

    @Test func aBracketPastTheEndDoesNothing() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.stepThread(by: 1))
        await model.settled()
        #expect(model.shown == 7)
        #expect(source.views.count == 1)
    }

    @Test func vPutsTheSourceInPlaceAndVAgainTakesItAway() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.viewSource)
        await model.settled()
        #expect(model.source == "Received: by example.com\r\n")
        #expect(model.showingSource)
        model.apply(.viewSource)
        #expect(!model.showingSource)
        model.apply(.viewSource)
        await model.settled()
        model.closeSource()
        #expect(model.source == nil, "Esc returns from the source too")
    }

    @Test func aSourceThatCannotBeReadSaysWhy() async {
        let source = Source()
        source.raw = .failure(SessionError.StoreUnavailable(message: "Not on this machine."))
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.viewSource)
        await model.settled()
        #expect(model.source?.contains("Not on this machine.") == true)
    }

    @Test func moreAndFindAreReportedSoBackClosesThemFirst() async {
        let source = Source()
        let (model, reports) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.showMore)
        model.apply(.findInMessage)
        model.apply(.closeMore)
        model.apply(.closeFind)
        #expect(reports.said.map { [$0.0, $0.1] } == [[true, false], [true, true], [false, true], [false, false]])
        #expect(!model.moreOpen && !model.find.isOpen)
    }

    @Test func findStepsAskTheViewToSearchAgain() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.findInMessage)
        model.setFindQuery("Friday")
        let first = model.find.request
        model.apply(.findNext)
        #expect(model.find.request != first)
        model.apply(.findPrevious)
        #expect(model.find.request?.backwards == true)
    }

    @Test func closingForgetsTheMessage() async {
        let source = Source()
        let (model, reports) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.showMore)
        model.closed()
        #expect(!model.isOpen)
        #expect(model.view == nil && model.document == nil && !model.moreOpen)
        #expect(reports.said.last.map { [$0.0, $0.1] } == [false, false])
        #expect(model.open(message: 7, index: 2, total: 9, mainWidth: 1440), "a fresh open again")
    }

    @Test func alwaysForThisSenderKeepsWhatIsShownAndAsksAgain() async {
        // The choice is the engine's to store (`always_treatment`); once
        // stored, the message is the sender's remembered choice, not `O`'s.
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.apply(.switchTreatment)
        await model.settled()
        var kept: [(String, TreatmentFfi)] = []
        model.keepForSender { kept.append(($0, $1)) }
        await model.settled()
        #expect(kept.map(\.0) == ["news@example.com"])
        #expect(kept.map(\.1) == [.appColours], "what was shown when it was pressed")
        #expect(model.chosen == nil)
        #expect(source.documents.last?.1 == nil, "asked again as the sender's own")
    }

    @Test func unsubscribingRecordsOneActivationForTheMessageOnScreen() async throws {
        // Only on deliberate activation: opening the message recorded
        // nothing, and pressing twice -- the banner's button, then `M` --
        // records once (#1585).
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        #expect(source.activated.isEmpty, "reading a message records nothing")
        #expect(model.unsubscribeState(of: 7) == .offered)

        model.unsubscribe()
        model.unsubscribe()
        try await Self.until { model.unsubscribeState(of: 7) == .left }
        #expect(model.unsubscribeState(of: 7) == .left, "the banner goes once it is recorded")
        #expect(source.activated == [7])
    }

    @Test func aRefusedUnsubscribeSaysWhyAndCanBePressedAgain() async throws {
        let source = Source()
        source.refusal = "There is no list to leave."
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()

        model.unsubscribe()
        try await Self.until { model.unsubscribeState(of: 7) != .leaving }
        #expect(model.unsubscribeState(of: 7) == .failed("There is no list to leave."))
        source.refusal = nil
        model.unsubscribe()
        try await Self.until { model.unsubscribeState(of: 7) == .left }
        #expect(source.activated == [7, 7])
    }

    @Test func unsubscribingWithNoMessageOnScreenDoesNothing() async {
        let source = Source()
        let (model, _) = Self.model(source)
        model.unsubscribe()
        await model.settled()
        #expect(source.activated.isEmpty)
    }

    @Test func closingTheWindowForgetsWhatWasLeft() async throws {
        let source = Source()
        let (model, _) = Self.model(source)
        model.open(message: 7, index: 2, total: 9, mainWidth: 1440)
        await model.settled()
        model.unsubscribe()
        try await Self.until { model.unsubscribeState(of: 7) == .left }
        model.closed()
        #expect(model.unsubscribeState(of: 7) == .offered)
    }
}
