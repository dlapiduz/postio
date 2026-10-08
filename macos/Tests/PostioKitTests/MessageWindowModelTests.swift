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
            return MessageWindowModelTests.document(shown: chosen ?? shown)
        }

        func rawSource(_ message: Int64) throws -> Data { try raw.get() }
    }

    nonisolated static func view(_ message: Int64, earlier: Int64? = nil, later: Int64? = nil) -> FocusMessageViewFfi {
        let action = FocusRowActionFfi(command: "dismiss_marker", label: "Dismiss")
        return FocusMessageViewFfi(
            message: message, subject: "Harbor API draft v3", position: "Message 1 of 3",
            thread: nil, earlier: earlier, later: later, labels: [], addLabel: action,
            fields: [], date: "Today, 15:22", marker: nil, dismiss: action, actions: [],
            more: nil, attachments: [])
    }

    nonisolated static func document(shown: TreatmentFfi) -> FocusReaderDocumentFfi {
        FocusReaderDocumentFfi(
            html: "<p>hello</p>", notice: nil, caveat: nil, treatmentShown: shown,
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
}
