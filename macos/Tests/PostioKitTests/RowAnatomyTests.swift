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
