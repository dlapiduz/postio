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

    @Test func theHintsCostTheListNoHeightAtAll() {
        // They used to cost a whole line of *every* row so that one row could
        // use it — a third of the height of the mail on screen, empty
        // everywhere the cursor was not. Seen by running the application and
        // holding it beside the canvas; every test passed, because they only
        // ever compared the densities to each other.
        for density in [DensityFfi.airy, .comfortable, .compact] {
            #expect(
                MessageRowCell.preferredHeight(for: density, reservingHints: false)
                    == MessageRowCell.preferredHeight(for: density, reservingHints: true),
                "\(density) still pays for a line it draws once"
            )
        }
    }

    @Test func aRowIsThreeLinesAndItsPaddingAndNothingElse() {
        // The absolute check the relative ones could not make. A row is the
        // sender, the subject, the snippet, the gaps between them and the
        // padding — and at 13pt that is under 80pt, not the 97 it was.
        #expect(MessageRowCell.preferredHeight(for: .airy) < 80)
        #expect(MessageRowCell.preferredHeight(for: .comfortable) < 72)
        // Compact drops the snippet, so it is two lines.
        #expect(MessageRowCell.preferredHeight(for: .compact) < 50)
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

/// The selected row is Postio's, not AppKit's (user report: "selecting a
/// message looks off").
///
/// The design system says *"airy rows, a 3px steel edge when selected"*, and
/// GTK has drawn that since it had rows. macOS drew `NSTableView`'s
/// system-blue fill, because `generate_swift` emitted no selection colour at
/// all — the row-state tokens are *derived* from `:root` rather than declared
/// in it, so the emitter's loop never saw them and there was nothing to draw
/// with.
@MainActor
@Suite struct SelectedRowTests {
    @Test func theSelectionTokensReachedSwift() {
        // The gap itself. Before this they did not exist on this platform,
        // and no amount of correct drawing code could have helped.
        #expect(PostioTokens.colorSelectedBg.alphaComponent > 0)
        #expect(PostioTokens.colorSelectedStrongBg.alphaComponent > 0)
    }

    @Test func theSelectedTintIsNotTheSystemHighlight() {
        // The point of the report: what was drawn was macOS's blue, which is
        // not the canvas's steel and never will be.
        let ours = PostioTokens.colorSelectedBg.usingColorSpace(.sRGB)
        let system = NSColor.selectedContentBackgroundColor.usingColorSpace(.sRGB)

        #expect(ours != system)
    }

    @Test func theEdgeIsTheThreePixelsTheCanvasNames() {
        #expect(MessageRowView.edge == 3)
    }

    /// The same measurement in both appearances.
    ///
    /// `colorSelectedBg` is a different colour in each — a 12% tint of the
    /// accent in light, the accent's deep step in dark — so an edge that is
    /// visible in one and not the other is a real defect, not a test detail.
    private func edgeIsVisible(in appearance: NSAppearance.Name) -> Bool {
        let view = MessageRowView(frame: NSRect(x: 0, y: 0, width: 200, height: 40))
        view.selectionHighlightStyle = .regular
        view.isSelected = true

        let image = NSImage(size: view.bounds.size)
        image.lockFocus()
        NSAppearance(named: appearance)?.performAsCurrentDrawingAppearance {
            // An opaque backdrop first, because that is what a person sees:
            // in the light appearance the tint is a **12% alpha** accent, so
            // its RGB is identical to the edge's and only its alpha differs.
            // Measured against transparency the two look the same colour and
            // the edge vanishes — which is a fact about reading a bitmap, not
            // about the row.
            PostioTokens.colorSurface.setFill()
            view.bounds.fill()
            view.drawSelection(in: view.bounds)
        }
        image.unlockFocus()

        guard let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!),
            let edge = bitmap.colorAt(x: 1, y: 20)?.usingColorSpace(.sRGB),
            let body = bitmap.colorAt(x: 100, y: 20)?.usingColorSpace(.sRGB)
        else { return false }
        let dr = edge.redComponent - body.redComponent
        let dg = edge.greenComponent - body.greenComponent
        let db = edge.blueComponent - body.blueComponent
        return (dr * dr + dg * dg + db * db).squareRoot() > 0.1
    }

    @Test func theEdgeIsVisibleInBothAppearances() {
        #expect(edgeIsVisible(in: .darkAqua), "no edge in dark")
        #expect(edgeIsVisible(in: .aqua), "no edge in light")
    }

    @Test func aRowDrawsItsOwnSelectionRatherThanInheritingOne() {
        // `drawSelection` is overridden, so AppKit's fill never runs. Asserted
        // by drawing into a bitmap and finding the accent edge down the
        // leading side — the thing a person actually sees.
        let view = MessageRowView(frame: NSRect(x: 0, y: 0, width: 200, height: 40))
        view.selectionHighlightStyle = .regular
        view.isSelected = true

        // Pinned, and this is the whole reason the test flaked on CI: the
        // selection tint is an `NSColor` with a dynamic provider, so it
        // resolves against whatever drawing appearance happens to be current.
        // On a desktop that is the app's; in a headless test process it is
        // whatever AppKit defaults to, and the tint and the accent can land
        // close enough together that no edge is measurable. Naming the
        // appearance makes the composite the same on any machine.
        let image = NSImage(size: view.bounds.size)
        image.lockFocus()
        NSAppearance(named: .darkAqua)?.performAsCurrentDrawingAppearance {
            PostioTokens.colorSurface.setFill()
            view.bounds.fill()
            view.drawSelection(in: view.bounds)
        }
        image.unlockFocus()

        let bitmap = NSBitmapImageRep(data: image.tiffRepresentation!)!
        let edge = bitmap.colorAt(x: 1, y: 20)!.usingColorSpace(.sRGB)!
        let body = bitmap.colorAt(x: 100, y: 20)!.usingColorSpace(.sRGB)!

        // Two things, and neither pins an exact pixel: the edge composites
        // over the tint beneath it, and an assertion on the resulting value
        // would break the next time either colour is retuned — which is
        // precisely what the canvas is *for*.
        //
        // What must be true is that there is an edge at all, and that it is
        // the accent rather than the system's. Distance is measured against
        // both candidates and the nearer one has to be ours.
        func distance(_ a: NSColor, _ b: NSColor) -> CGFloat {
            let dr = a.redComponent - b.redComponent
            let dg = a.greenComponent - b.greenComponent
            let db = a.blueComponent - b.blueComponent
            return (dr * dr + dg * dg + db * db).squareRoot()
        }
        let accent = PostioTokens.colorAccent.usingColorSpace(.sRGB)!
        let system = NSColor.selectedContentBackgroundColor.usingColorSpace(.sRGB)!

        #expect(
            distance(edge, body) > 0.1,
            "there is no edge: the leading pixels match the row's fill"
        )
        #expect(
            distance(edge, accent) < distance(edge, system),
            "the edge is nearer the system highlight than the accent: \(edge)"
        )
    }
}
