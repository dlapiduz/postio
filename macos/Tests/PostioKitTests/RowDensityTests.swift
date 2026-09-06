import AppKit
import PostioFFI
import Testing

@testable import PostioKit

/// What row density actually does to a row.
///
/// The settings pane can write `density` all it likes; until something here
/// changes, a Mac user picks Compact and watches nothing happen. These assert
/// the drawn row rather than the value that was handed to it — a cell that
/// merely *stored* a density would pass a test written the other way and look
/// identical on screen.
@MainActor
@Suite struct RowDensityTests {
    private func presentation(preview: String = "…the numbers we discussed on") -> RowPresentation {
        RowPresentation(
            sender: "ada@example.com",
            subject: "Quarterly figures",
            preview: preview,
            unread: false,
            flagged: false,
            threadBadge: nil,
            isPlaceholder: false,
            snippet: nil
        )
    }

    private func laidOut(_ density: DensityFfi) -> MessageRowCell {
        let cell = MessageRowCell(frame: NSRect(x: 0, y: 0, width: 420, height: 200))
        cell.density = density
        cell.show(presentation())
        cell.layoutSubtreeIfNeeded()
        return cell
    }

    @Test func eachStepDownInDensityMakesTheRowShorter() {
        let airy = MessageRowCell.preferredHeight(for: .airy)
        let snug = MessageRowCell.preferredHeight(for: .comfortable)
        let compact = MessageRowCell.preferredHeight(for: .compact)

        #expect(airy > snug, "airy \(airy) is not taller than snug \(snug)")
        #expect(snug > compact, "snug \(snug) is not taller than compact \(compact)")
    }

    @Test func theTightestDensityDropsTheSnippetAndTheOthersKeepIt() {
        // Compact exists to answer "how many subjects fit", and the snippet is
        // the line that costs the most and answers it least. This is what the
        // density *is*, so both frontends drop it at the same setting.
        #expect(laidOut(.compact).previewIsHiddenForTesting)
        #expect(!laidOut(.airy).previewIsHiddenForTesting)
        #expect(!laidOut(.comfortable).previewIsHiddenForTesting)
    }

    @Test func aRowIsStillTallEnoughForWhatItDrawsAtEveryDensity() {
        // The bug this guards is the one that shipped once already: a row
        // height chosen as a literal, too small for its contents, clipping
        // every row after the first in a way no test could see.
        for density in [DensityFfi.airy, .comfortable, .compact] {
            let cell = laidOut(density)
            let needed = cell.fittingSize.height
            let declared = MessageRowCell.preferredHeight(for: density)
            #expect(
                declared >= needed,
                "\(density) declares \(declared) but its content needs \(needed)"
            )
        }
    }

    @Test func theMetricsAreTheOnesTheOtherFrontendDrawsBy() {
        // Not a second table of numbers: `postio_ui::row::Metrics` is what GTK
        // lays out with, and "compact" has to mean one thing.
        #expect(rowMetrics(density: .compact).snippet == false)
        #expect(rowMetrics(density: .airy).padY > rowMetrics(density: .compact).padY)
    }
}

/// The density reaching the list, rather than sitting in a value nobody reads.
@MainActor
@Suite struct RowDensityWiringTests {
    @Test func theControllerHandsItsDensityToEveryCellItMakesOrReuses() {
        // The bug this exists for: a controller that stored the density and
        // configured only *new* cells leaves every recycled row at the old
        // one, so scrolling shows two densities at once.
        let controller = MessageTableController(source: StubRowSource(rowCount: 0))
        controller.density = .compact

        let fresh = controller.cell(reusing: nil)
        #expect(fresh.density == .compact)

        let stale = MessageRowCell()
        stale.density = .airy
        #expect(controller.cell(reusing: stale).density == .compact)
    }

    @Test func theTableIsToldTheHeightTheDensityAsksFor() {
        let controller = MessageTableController(source: StubRowSource(rowCount: 0))
        // Hints are on by default and their line is reserved on every row, so
        // the table's height is the one that includes it.
        controller.density = .compact
        #expect(
            controller.rowHeight
                == MessageRowCell.preferredHeight(for: .compact, reservingHints: true))

        controller.density = .airy
        #expect(
            controller.rowHeight
                == MessageRowCell.preferredHeight(for: .airy, reservingHints: true))
    }

    @Test func theViewBuildsATableAtTheDensitysHeight() {
        // The last hop this suite can reach: `Engine` sets the controller's
        // density, and `MessageListView` is what turns that into a real
        // `NSTableView`. A controller that knew its height while the table
        // kept the old one is the "built, tested, never mounted" shape, and
        // it is the one this project keeps finding.
        let controller = MessageTableController(source: StubRowSource(rowCount: 0))
        controller.density = .compact
        let scroll = MessageListView.makeTable(controller: controller)
        let table = scroll.documentView as? NSTableView
        #expect(table?.rowHeight == controller.rowHeight)
    }
}

/// The sentence under the density control.
@MainActor
@Suite struct DensityReadoutTests {
    @Test func theReadoutIsTheHeightTheListActuallyUses() {
        // The number a person reads has to be the number the rows are drawn
        // at. GTK measures a real row for this rather than tabulating; so
        // does the pane, from the same cell the list makes.
        for density in [DensityFfi.airy, .comfortable, .compact] {
            let controller = MessageTableController(source: StubRowSource(rowCount: 0))
            controller.density = density
            let shown = Int(
                MessageRowCell.preferredHeight(
                    for: density, reservingHints: controller.ui.showKeyHints))
            #expect(CGFloat(shown) == controller.rowHeight.rounded(.down))
        }
    }
}
