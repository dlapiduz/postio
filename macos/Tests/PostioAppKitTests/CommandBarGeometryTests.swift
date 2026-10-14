import CoreGraphics
import Testing

@testable import PostioKit

/// Where the search field grows and where the panel hangs (specs/010-focus-
/// search T054; design §2 "Opening and layout"): the field grows leftward to
/// 860 wide, its right edge 12 from the window's; the panel hangs 6 below
/// it with the same left edge and width, and stays inside its window.
/// Screen coordinates, the origin at the bottom left, as AppKit has them.
struct CommandBarGeometryTests {
    /// The field as the toolbar lays it out at `width` for a window at the
    /// screen's origin, 900 tall: 34 tall, its top 9 under the window's.
    static func field(in window: CGRect) -> CGRect {
        let width = CommandBarGeometry.fieldWidth(window: window.width)
        return CGRect(
            x: window.maxX - CommandBarGeometry.edge - width, y: window.maxY - 9 - 34,
            width: width, height: 34)
    }

    @Test func atFourteenFortyTheFieldIs860AndThePanelHangsSixBelowIt() {
        let window = CGRect(x: 0, y: 0, width: 1440, height: 900)
        #expect(CommandBarGeometry.fieldWidth(window: 1440) == 860)
        let field = Self.field(in: window)
        #expect(field.maxX == 1428, "its right edge 12 from the window's")
        let frame = CommandBarGeometry.frame(field: field, window: window, height: 390)
        #expect(frame.minX == field.minX, "the field's left edge")
        #expect(frame.width == field.width, "and its width")
        #expect(frame.maxY == field.minY - 6, "6 below it")
        #expect(frame.height == 390)
    }

    @Test func atTenTwentyFourTheFieldGivesWayToTheToolbarsLeftItems() {
        let window = CGRect(x: 200, y: 100, width: 1024, height: 700)
        let width = CommandBarGeometry.fieldWidth(window: 1024)
        #expect(width < 860)
        #expect(width == 1024 - CommandBarGeometry.edge - CommandBarGeometry.leading)
        let field = Self.field(in: window)
        let frame = CommandBarGeometry.frame(field: field, window: window, height: 390)
        #expect(frame.minX == field.minX)
        #expect(frame.width == field.width)
        #expect(frame.maxY == field.minY - 6)
    }

    @Test func thePanelIsShortenedToEndAboveItsWindowsBottom() {
        let window = CGRect(x: 100, y: 100, width: 1024, height: 400)
        let field = Self.field(in: window)
        let frame = CommandBarGeometry.frame(field: field, window: window, height: 900)
        #expect(frame.minY >= window.minY + CommandBarGeometry.margin)
        #expect(frame.maxY == field.minY - 6)
    }
}
