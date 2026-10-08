import PostioFFI
import Testing

@testable import PostioKit

/// A Focus row as the Mac draws it, built from what the FFI hands over
/// (specs/009-focus-macos T030, screens 01 and 02).
///
/// Everything a row *says* is the engine's -- sender, subject, the
/// marker's chip, date and quote, the actions' words. What is decided
/// here is only how it is laid out, and these tests pin that: which of
/// the three layouts a row takes, that no more than two pills reach the
/// line, that an action's keycap is the user's binding rather than a
/// letter typed into Swift, and that unread is bold.
@Suite struct FocusRowModelTests {
    /// The keymap as the registry would answer it.
    static let keymap: [String: String] = [
        "reply": "e",
        "accept_invite": "y",
        "decline_invite": "Y",
        "snooze": "s",
        "capture_task": "t",
    ]

    static func binding(_ command: String) -> String? { keymap[command] }

    static func row(
        kind: FocusRowKindFfi = .conversation,
        id: Int64 = 1,
        unread: Bool = false,
        heading: String = "Today · Saturday 26 September",
        pills: [LabelPillFfi] = [],
        marker: MarkerLineFfi? = nil
    ) -> FocusRowFfi {
        FocusRowFfi(
            kind: kind,
            id: id,
            thread: kind == .digest ? nil : id,
            threads: kind == .digest ? [] : [id],
            sender: "Ada Moreno",
            subject: "Re: Atlas Q3 budget, final numbers",
            preview: "Hi, the final Q3 numbers are in the attached sheet.",
            time: "15:51",
            dayHeading: heading,
            unread: unread,
            countBadge: nil,
            hasAttachments: false,
            sendState: nil,
            pills: pills,
            marker: marker,
            writes: false
        )
    }

    static let question = MarkerLineFfi(
        chip: "Question",
        date: nil,
        quote: "Can you approve these by Friday so finance can close the quarter?",
        status: nil,
        actions: [FocusRowActionFfi(command: "reply", label: "Reply")]
    )

    // MARK: layout

    @Test func aConversationWithNoMarkerIsOneLine() {
        let model = FocusRowModel(Self.row(), binding: Self.binding)
        #expect(model.layout == .oneLine)
    }

    @Test func aMarkerGrowsTheSecondLine() {
        let model = FocusRowModel(Self.row(marker: Self.question), binding: Self.binding)
        #expect(model.layout == .twoLine)
        #expect(model.marker?.chip == "Question")
    }

    @Test func aReminderWithItsMarkerIsTwoLinesLikeAConversation() {
        let marker = MarkerLineFfi(
            chip: "No reply", date: "since Sat 26 Sep", quote: nil, status: nil,
            actions: [FocusRowActionFfi(command: "reply", label: "Reply")])
        let model = FocusRowModel(Self.row(kind: .reminder, marker: marker), binding: Self.binding)
        #expect(model.layout == .twoLine)
    }

    @Test func aDigestIsItsOwnLayout() {
        let model = FocusRowModel(Self.row(kind: .digest), binding: Self.binding)
        #expect(model.layout == .digest)
    }

    @Test func eachLayoutHasAFixedHeightSoNothingIsMeasured() {
        // Screen 01: a one-line row is 40pt, a marked row 72pt, and a day
        // heading adds its 32pt band above the first row of its day.
        #expect(FocusRowMetrics.height(.oneLine, headed: false) == 40)
        #expect(FocusRowMetrics.height(.digest, headed: false) == 40)
        #expect(FocusRowMetrics.height(.twoLine, headed: false) == 72)
        #expect(FocusRowMetrics.height(.oneLine, headed: true) == 72)
        #expect(FocusRowMetrics.height(.twoLine, headed: true) == 104)
    }

    // MARK: pills

    @Test func atMostTwoPillsReachTheLine() {
        let pills = [
            LabelPillFfi(name: "Atlas", color: "#d9822b"),
            LabelPillFfi(name: "Home", color: nil),
            LabelPillFfi(name: "Kids", color: "#c23030"),
        ]
        let model = FocusRowModel(Self.row(pills: pills), binding: Self.binding)
        #expect(model.pills.map(\.name) == ["Atlas", "Home"])
    }

    @Test func aPillKeepsTheColourItsLabelWasGivenAndNoneWhenItHasNone() {
        let pills = [
            LabelPillFfi(name: "Atlas", color: "#ff8000"),
            LabelPillFfi(name: "Home", color: nil),
        ]
        let model = FocusRowModel(Self.row(pills: pills), binding: Self.binding)
        #expect(model.pills[0].colour == LabelColour(red: 1, green: 128.0 / 255, blue: 0))
        #expect(model.pills[1].colour == nil)
        // Not a colour: drawn as the plain dot rather than guessed at.
        #expect(LabelColour(hex: "orange") == nil)
        #expect(LabelColour(hex: "#12345") == nil)
    }

    // MARK: keycaps

    @Test func anActionsKeycapIsTheUsersBindingNotALiteral() {
        let model = FocusRowModel(Self.row(marker: Self.question), binding: Self.binding)
        #expect(model.marker?.actions.map(\.label) == ["Reply"])
        #expect(model.marker?.actions.map(\.cap) == ["e"])

        // Rebinding reply moves the cap with it: nothing in the row spells
        // `e` itself.
        let rebound = FocusRowModel(Self.row(marker: Self.question)) { command in
            command == "reply" ? "r" : nil
        }
        #expect(rebound.marker?.actions.map(\.cap) == ["r"])
    }

    @Test func anActionWithNoKeyDrawsNoCap() {
        let model = FocusRowModel(Self.row(marker: Self.question)) { _ in nil }
        #expect(model.marker?.actions.first?.cap == .some(nil))
    }

    @Test func anInvitesTwoAnswersKeepTheirOwnKeys() {
        let invite = MarkerLineFfi(
            chip: "Invite", date: "Tue 29 Sep · 10:00–10:45", quote: nil, status: nil,
            actions: [
                FocusRowActionFfi(command: "accept_invite", label: "Accept"),
                FocusRowActionFfi(command: "decline_invite", label: "Decline"),
            ])
        let model = FocusRowModel(Self.row(marker: invite), binding: Self.binding)
        // The keymap's spelling (spec 007 C22): `y` and `Y` are two keys,
        // and the cap must not fold them into one.
        #expect(model.marker?.actions.map(\.cap) == ["y", "Y"])
    }

    @Test func aCapIsSpelledAsTheKeymapSpellsItWithGlyphsForChords() {
        #expect(KeyCapSpelling.cap("e") == "e")
        #expect(KeyCapSpelling.cap("Y") == "Y")
        #expect(KeyCapSpelling.cap("!") == "!")
        // A sequence is one cap, as the strip's `g o` is on screen 01.
        #expect(KeyCapSpelling.cap("g o") == "g o")
        #expect(KeyCapSpelling.cap("cmd+k") == "⌘K")
        #expect(KeyCapSpelling.cap(nil) == nil)
    }

    // MARK: the marker's words

    @Test func theQuoteIsDrawnVerbatimInsideQuotationMarks() {
        let model = FocusRowModel(Self.row(marker: Self.question), binding: Self.binding)
        #expect(
            model.marker?.quoted
                == "\u{201c}Can you approve these by Friday so finance can close the quarter?\u{201d}"
        )
    }

    @Test func aStatusStandsWhereTheActionsWouldBe() {
        let answered = MarkerLineFfi(
            chip: "Invite", date: "Tue 29 Sep · 10:00–10:45", quote: nil, status: "Accepted",
            actions: [])
        let model = FocusRowModel(Self.row(marker: answered), binding: Self.binding)
        #expect(model.marker?.status == "Accepted")
        #expect(model.marker?.actions.isEmpty == true)
    }

    // MARK: weight

    @Test func unreadIsBold() {
        #expect(FocusRowModel(Self.row(unread: true), binding: Self.binding).bold)
        #expect(!FocusRowModel(Self.row(unread: false), binding: Self.binding).bold)
    }

    @Test func aScreenReaderHearsTheRowAndItsMarker() {
        let model = FocusRowModel(
            Self.row(unread: true, marker: Self.question), binding: Self.binding)
        #expect(
            model.accessibilityLabel
                == "Ada Moreno, Re: Atlas Q3 budget, final numbers, "
                + "Hi, the final Q3 numbers are in the attached sheet., unread, "
                + "Question: \u{201c}Can you approve these by Friday so finance can close the quarter?\u{201d}"
        )
    }
}

/// The list over the FFI: how many rows, which of them start a day, and
/// what a page arriving changes (T031).
@MainActor
@Suite struct FocusListModelTests {
    /// Rows by position, `nil` while "its page is on its way".
    final class Rows: FocusRowSource {
        var rows: [FocusRowFfi?]
        var asked: [UInt32] = []
        init(_ rows: [FocusRowFfi?]) { self.rows = rows }
        var focusRowCount: UInt32 { UInt32(rows.count) }
        func focusRow(at position: UInt32) -> FocusRowFfi? {
            asked.append(position)
            return Int(position) < rows.count ? rows[Int(position)] : nil
        }
    }

    static let today = "Today · Saturday 26 September"
    static let yesterday = "Yesterday · Friday 25 September"

    static func row(_ id: Int64, _ heading: String, marker: MarkerLineFfi? = nil) -> FocusRowFfi {
        FocusRowModelTests.row(id: id, heading: heading, marker: marker)
    }

    static func list(_ rows: [FocusRowFfi?]) -> (FocusListModel, Rows) {
        let source = Rows(rows)
        let model = FocusListModel(source: source, binding: FocusRowModelTests.binding)
        model.reset(total: source.focusRowCount)
        return (model, source)
    }

    @Test func theCountIsTheEnginesNotAnArrayHeldHere() {
        let (model, _) = Self.list([Self.row(1, Self.today), Self.row(2, Self.today)])
        #expect(model.count == 2)
        model.reset(total: 7)
        #expect(model.count == 7)
    }

    @Test func aDayHeadingIsDrawnOverTheFirstRowOfEachDayOnly() {
        let (model, _) = Self.list([
            Self.row(1, Self.today),
            Self.row(2, Self.today),
            Self.row(3, Self.yesterday),
            Self.row(4, Self.yesterday),
        ])
        #expect((0..<4).map { model.row(at: $0)?.heading } == [Self.today, nil, Self.yesterday, nil])
    }

    @Test func aRowWhosePageHasNotArrivedIsAPlaceholder() {
        let (model, _) = Self.list([Self.row(1, Self.today), nil])
        #expect(model.row(at: 1) == nil)
        #expect(model.row(at: 5) == nil)
    }

    @Test func aHeightIsAnsweredWithoutAskingTheEngine() {
        // `heightOfRow` is asked for every row in the table; it must never
        // reach the FFI, or a 10k-row inbox fetches every page to lay out.
        let (model, source) = Self.list([Self.row(1, Self.today), Self.row(2, Self.today)])
        source.asked = []
        _ = (0..<2).map { model.height(at: $0) }
        #expect(source.asked.isEmpty)
        // Before anything is known the first row is a headed one-liner and
        // the rest one-liners.
        #expect(model.height(at: 0) == FocusRowMetrics.height(.oneLine, headed: true))
        #expect(model.height(at: 1) == FocusRowMetrics.height(.oneLine, headed: false))
    }

    @Test func aPageArrivingLearnsItsRowsShapesAndSaysWhichToRedraw() {
        let marked = FocusRowModelTests.question
        let (model, _) = Self.list([
            Self.row(1, Self.today),
            Self.row(2, Self.today, marker: marked),
            Self.row(3, Self.yesterday),
        ])
        let changed = model.pageArrived(0)
        #expect(changed.contains(integersIn: 0..<3))
        #expect(model.height(at: 0) == FocusRowMetrics.height(.oneLine, headed: true))
        #expect(model.height(at: 1) == FocusRowMetrics.height(.twoLine, headed: false))
        #expect(model.height(at: 2) == FocusRowMetrics.height(.oneLine, headed: true))
    }

    @Test func aPageArrivingAfterTheNextOneRedrawsTheNextPagesFirstRow() {
        // Rows 50 and 49 straddle a page boundary: whether 50 starts a day
        // is only known once 49 is.
        let size = Int(FocusListModel.pageSize)
        var rows: [FocusRowFfi?] = Array(repeating: nil, count: size)
        rows.append(Self.row(51, Self.today))
        let (model, source) = Self.list(rows)
        _ = model.pageArrived(1)
        #expect(model.row(at: size)?.heading == nil)

        source.rows = (0..<size).map { Self.row(Int64($0 + 1), Self.today) } + [Self.row(51, Self.today)]
        let changed = model.pageArrived(0)
        #expect(changed.contains(size))
        #expect(model.row(at: size)?.heading == nil)

        source.rows[size - 1] = Self.row(50, Self.yesterday)
        _ = model.pageArrived(0)
        #expect(model.row(at: size)?.heading == Self.today)
    }

    @Test func theListOpensWithTheCursorOnItsFirstRow() {
        // C30: every list opens with the cursor on its first row.
        let (model, _) = Self.list([Self.row(1, Self.today), Self.row(2, Self.today)])
        #expect(model.cursor == 0)
        model.reset(total: 0)
        #expect(model.cursor == nil)
        model.reset(total: 3)
        #expect(model.cursor == 0)
    }

    @Test func aListChangingForgetsWhatItLearned() {
        let (model, source) = Self.list([Self.row(1, Self.today, marker: FocusRowModelTests.question)])
        _ = model.pageArrived(0)
        #expect(model.height(at: 0) == FocusRowMetrics.height(.twoLine, headed: true))
        source.rows = [Self.row(9, Self.today)]
        model.reset(total: 1)
        #expect(model.height(at: 0) == FocusRowMetrics.height(.oneLine, headed: true))
    }

    @Test func aKeyRetunedIsAskedForAgain() {
        var keys = ["reply": "e"]
        let source = Rows([Self.row(1, Self.today, marker: FocusRowModelTests.question)])
        let model = FocusListModel(source: source) { keys[$0] }
        model.reset(total: 1)
        #expect(model.row(at: 0)?.marker?.actions.first?.cap == "e")
        keys["reply"] = "r"
        // Cached until the keymap says it changed: a binding lookup clones
        // the keymap on the other side, and a table asks per visible row.
        #expect(model.row(at: 0)?.marker?.actions.first?.cap == "e")
        model.keymapChanged()
        #expect(model.row(at: 0)?.marker?.actions.first?.cap == "r")
    }
}
