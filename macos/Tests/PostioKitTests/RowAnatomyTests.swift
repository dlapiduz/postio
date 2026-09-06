import AppKit
import PostioFFI
import Testing

@testable import PostioKit

/// Canvas 1b's row, on the platform that was missing most of it.
///
/// The row drew sender, subject and preview and nothing else — no avatar
/// chip, no time. A mail list that does not say *when* is missing something
/// no setting covers, and two of `[ui]`'s flags had nothing to act on.
@MainActor
@Suite struct RowAnatomyTests {
    private func presentation(
        initials: String = "AL",
        time: String = "09:14"
    ) -> RowPresentation {
        RowPresentation(
            sender: "Ada Lovelace",
            subject: "Quarterly figures",
            preview: "…the numbers we discussed on",
            unread: false,
            flagged: false,
            threadBadge: nil,
            isPlaceholder: false,
            snippet: nil,
            initials: initials,
            time: time
        )
    }

    private func laidOut(_ appearance: AppearanceFfi) -> MessageRowCell {
        let cell = MessageRowCell(frame: NSRect(x: 0, y: 0, width: 420, height: 200))
        cell.ui = appearance
        cell.show(presentation())
        cell.layoutSubtreeIfNeeded()
        return cell
    }

    private func appearance(
        density: DensityFfi = .airy,
        avatars: Bool = true
    ) -> AppearanceFfi {
        AppearanceFfi(
            density: density,
            theme: .system,
            showHoverActions: true,
            showKeyHints: true,
            senderAvatars: avatars
        )
    }

    @Test func theRowSaysWhenItArrived() {
        // Nothing configures this and nothing showed it: the list simply had
        // no time column at all.
        #expect(laidOut(appearance()).drawnForTesting.time == "09:14")
    }

    @Test func theAvatarChipShowsTheSharedInitials() {
        // The letters come from `postio_ui::row::initials` through the
        // boundary, so a mailing list abbreviates the same on both platforms
        // rather than one of them shrugging.
        #expect(laidOut(appearance()).drawnForTesting.initials == "AL")
    }

    @Test func turningSenderAvatarsOffRemovesTheChipRatherThanBlankingIt() {
        // The setting existed and did nothing. "Off" has to mean the row gets
        // the space back, not that it draws an empty circle.
        #expect(laidOut(appearance(avatars: false)).avatarIsHiddenForTesting)
        #expect(!laidOut(appearance(avatars: true)).avatarIsHiddenForTesting)
    }

    @Test func theChipIsTheSizeTheDensityAsksFor() {
        // `Metrics.avatar` is 30/26/22, shared with GTK. A chip that ignored
        // it would make the row taller than the density budgeted for.
        for density in [DensityFfi.airy, .comfortable, .compact] {
            let cell = laidOut(appearance(density: density))
            let expected = CGFloat(rowMetrics(density: density).avatar)
            #expect(cell.avatarSizeForTesting == expected, "\(density)")
        }
    }

    @Test func onlyTheFocusedRowTeachesTheKeyboard() {
        // The hints exist to teach the keyboard by using the app. On every
        // row at once they are noise; on none of them the keyboard has to be
        // read about instead.
        let hints = [RowHintFfi(key: "e", label: "reply")]

        let focused = laidOut(appearance())
        focused.hints = hints
        focused.focused = true
        focused.layoutSubtreeIfNeeded()
        #expect(focused.hintsForTesting == "e reply")

        let other = laidOut(appearance())
        other.hints = hints
        other.focused = false
        #expect(other.hintsForTesting.isEmpty)
    }

    @Test func turningKeyHintsOffLeavesEveryBindingInForce() {
        // #422: off only stops the row naming them, for someone who already
        // knows the keyboard. Nothing about dispatch changes, so the only
        // observable is that the row stops saying so.
        let cell = laidOut(appearance())
        cell.ui.showKeyHints = false
        cell.hints = [RowHintFfi(key: "e", label: "reply")]
        cell.focused = true
        #expect(cell.hintsForTesting.isEmpty)
    }

    @Test func aRowIsStillTallEnoughOnceTheChipIsInIt() {
        for density in [DensityFfi.airy, .comfortable, .compact] {
            let cell = laidOut(appearance(density: density))
            #expect(MessageRowCell.preferredHeight(for: density) >= cell.fittingSize.height)
        }
    }
}

/// The `[ui]` table reaching the cells that draw by it.
@MainActor
@Suite struct RowAnatomyWiringTests {
    @Test func theControllerHandsTheWholeTableToEveryCell() {
        // Not just the density: `sender_avatars` decides whether the chip is
        // there at all, and a cell that got the height but not the flag draws
        // a row nobody asked for.
        let controller = MessageTableController(source: StubRowSource(rowCount: 0))
        controller.ui = AppearanceFfi(
            density: .compact,
            theme: .dark,
            showHoverActions: false,
            showKeyHints: false,
            senderAvatars: false
        )

        #expect(controller.cell(reusing: nil).ui.senderAvatars == false)

        let stale = MessageRowCell()
        stale.ui.senderAvatars = true
        #expect(controller.cell(reusing: stale).ui.senderAvatars == false)
    }
}

/// The hint line's space, which every row pays for and one row uses.
@MainActor
@Suite struct KeyHintHeightTests {
    @Test func theFocusedRowFitsItsHintsWithoutGrowing() {
        // `NSTableView` draws a fixed height, so a row that grew when it took
        // the cursor would clip instead. The space is reserved on every row.
        for density in [DensityFfi.airy, .comfortable, .compact] {
            let cell = MessageRowCell(frame: NSRect(x: 0, y: 0, width: 420, height: 200))
            cell.ui = AppearanceFfi(
                density: density, theme: .system, showHoverActions: true,
                showKeyHints: true, senderAvatars: true
            )
            cell.hints = [RowHintFfi(key: "e", label: "reply"), RowHintFfi(key: "a", label: "archive")]
            cell.focused = true
            cell.show(
                RowPresentation(
                    sender: "Ada", subject: "Figures", preview: "…numbers",
                    unread: false, flagged: false, threadBadge: nil,
                    isPlaceholder: false, initials: "AL", time: "09:14"
                )
            )
            cell.layoutSubtreeIfNeeded()

            let declared = MessageRowCell.preferredHeight(for: density, reservingHints: true)
            #expect(declared >= cell.fittingSize.height, "\(density) clips its hints")
        }
    }

    @Test func turningHintsOffGivesTheListTheSpaceBack() {
        for density in [DensityFfi.airy, .comfortable, .compact] {
            #expect(
                MessageRowCell.preferredHeight(for: density, reservingHints: false)
                    < MessageRowCell.preferredHeight(for: density, reservingHints: true)
            )
        }
    }
}

/// The cursor moving, and the two rows that have to be redrawn when it does.
@MainActor
@Suite struct HintRepaintTests {
    private func controller() -> MessageTableController {
        let controller = MessageTableController(source: StubRowSource(rowCount: 10))
        let scroll = MessageListView.makeTable(controller: controller)
        controller.tableView = scroll.documentView as? NSTableView
        controller.tableView?.reloadData()
        return controller
    }

    @Test func movingTheCursorRedrawsTheRowThatLostTheHintsAndTheOneThatGainedThem() {
        // Nothing else repaints on a cursor move -- the list is windowed and
        // reloads when a page lands, not when the selection changes. So
        // without this the hints stay on the row the cursor left, which is
        // worse than not drawing them: it points at the wrong message.
        let controller = controller()
        controller.showCursor(on: 3)
        controller.repaintedForHintsForTesting = []

        controller.showCursor(on: 4)
        #expect(controller.repaintedForHintsForTesting.sorted() == [3, 4])
    }

    @Test func aCursorMoveWithNothingFocusedBeforeRedrawsOnlyTheNewRow() {
        let controller = controller()
        controller.showCursor(on: nil)
        controller.repaintedForHintsForTesting = []

        controller.showCursor(on: 2)
        #expect(controller.repaintedForHintsForTesting == [2])
    }

    // -- a conversation row names the people in it (#1265) -----------------

    @Test func aThreadRowNamesTheConversationRatherThanItsNewestSender() {
        // Every row in a folder stands for a conversation (ADR 0015), and the
        // canvas draws `Tessa Vaughn, Mara, Pinepoint` where a message row
        // draws one name. Drawing only the representative's sender loses the
        // one fact that tells two threads on the same subject apart.
        let row = RowFfi(
            id: 1,
            thread: 7,
            isThread: true,
            from: "Pinepoint Radon",
            fromAddress: "hello@pinepoint-radon.example",
            initials: "TV",
            subject: "Radon reduction",
            preview: "I am following up",
            receivedAt: 1_770_000_000,
            seen: false,
            flagged: false,
            answered: false,
            draft: false,
            hasAttachments: false,
            threadCount: 8,
            participants: "Tessa, Mara, Pinepoint"
        )

        let presentation = RowPresentation(row: row)

        #expect(presentation.sender == "Tessa, Mara, Pinepoint")
        #expect(presentation.threadBadge == "8")
    }

    @Test func aMessageRowStillNamesItsSender() {
        // A query view lists messages, not conversations, and a message row
        // carries no participants — the discriminator, not an empty field to
        // fall through.
        let row = RowFfi(
            id: 1,
            thread: 7,
            isThread: false,
            from: "Pinepoint Radon",
            fromAddress: "hello@pinepoint-radon.example",
            initials: "PR",
            subject: "Radon reduction",
            preview: "I am following up",
            receivedAt: 1_770_000_000,
            seen: true,
            flagged: false,
            answered: false,
            draft: false,
            hasAttachments: false,
            threadCount: 8,
            participants: ""
        )

        #expect(RowPresentation(row: row).sender == "Pinepoint Radon")
    }
}
