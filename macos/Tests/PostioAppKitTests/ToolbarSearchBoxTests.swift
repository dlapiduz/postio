import AppKit
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// The inbox toolbar's search field (specs/010-focus-search T054; design
/// §2 "Opening and layout"), in a real unified toolbar: `NSSearchToolbarItem`
/// ignored every width it was given and stayed about 327 wide (step 8's
/// note), so the field is Postio's own view, and these read where the
/// toolbar actually put it.
///
/// At rest it is 320 by 28 (McList.dc.html). Opened -- ⌘K or `/` -- it
/// grows leftward to 860, its right edge 12 from the window's, 34 tall,
/// ringed in the accent; the panel hangs from it with the same left edge
/// and width, 6 below.
@MainActor
@Suite struct ToolbarSearchBoxTests {
    @MainActor final class Delegate: NSObject, NSToolbarDelegate {
        let box = ToolbarSearchBox()
        static let compose = NSToolbarItem.Identifier("test.compose")
        static let search = NSToolbarItem.Identifier("test.search")

        func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
            [Self.compose, .flexibleSpace, Self.search]
        }

        func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
            toolbarDefaultItemIdentifiers(toolbar)
        }

        func toolbar(
            _ toolbar: NSToolbar, itemForItemIdentifier identifier: NSToolbarItem.Identifier,
            willBeInsertedIntoToolbar flag: Bool
        ) -> NSToolbarItem? {
            switch identifier {
            case Self.compose:
                let item = NSToolbarItem(itemIdentifier: identifier)
                item.image = NSImage(systemSymbolName: "square.and.pencil", accessibilityDescription: "Compose")
                item.label = "Compose"
                return item
            case Self.search:
                return box.item(identifier)
            default:
                return nil
            }
        }
    }

    /// A window `width` wide with the inbox's toolbar shape: Compose, a
    /// flexible space, the field.
    static func window(width: CGFloat) -> (NSWindow, Delegate) {
        let window = NSWindow(
            contentRect: NSRect(x: 40, y: 40, width: width, height: 700),
            styleMask: [.titled, .closable, .miniaturizable, .resizable, .fullSizeContentView],
            backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.titleVisibility = .hidden
        window.toolbarStyle = .unified
        let delegate = Delegate()
        let toolbar = NSToolbar(identifier: "ToolbarSearchBoxTests.\(Int(width))")
        toolbar.delegate = delegate
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
        window.toolbar = toolbar
        window.orderFront(nil)
        return (window, delegate)
    }

    /// The box's frame on screen, after the toolbar has laid it out.
    static func frame(of box: NSView, in window: NSWindow) -> CGRect {
        window.contentView?.superview?.layoutSubtreeIfNeeded()
        return window.convertToScreen(box.convert(box.bounds, to: nil))
    }

    @Test(arguments: [CGFloat(1440), CGFloat(1024)])
    func openedTheFieldGrowsLeftwardWithItsRightEdge12FromTheWindows(width: CGFloat) {
        let (window, delegate) = Self.window(width: width)
        defer { window.close() }
        let box = delegate.box
        let resting = Self.frame(of: box.frameView, in: window)

        box.open(true, windowWidth: width)
        let field = Self.frame(of: box.frameView, in: window)

        #expect(field.width == CommandBarGeometry.fieldWidth(window: width))
        #expect(width != 1440 || field.width == 860)
        #expect(window.frame.maxX - field.maxX == CommandBarGeometry.edge, "its right edge 12 from the window's")
        #expect(field.maxX == resting.maxX, "it grows leftward")
        #expect(field.height == ToolbarSearchBox.openHeight)
        #expect(field.midY == resting.midY, "on the toolbar's middle")
        #expect(box.ringed)

        let panel = CommandBarGeometry.frame(field: field, window: window.frame, height: 390)
        #expect(panel.minX == field.minX, "the panel hangs from the field's left edge")
        #expect(panel.width == field.width, "with its width")
        #expect(panel.maxY == field.minY - CommandBarGeometry.gap, "6 below it")
    }

    @Test func atRestItIs320By28AndClosingPutsItBack() {
        let (window, delegate) = Self.window(width: 1440)
        defer { window.close() }
        let box = delegate.box
        let resting = Self.frame(of: box.frameView, in: window)
        #expect(resting.width == CommandBarGeometry.restingWidth)
        #expect(resting.height == ToolbarSearchBox.restingHeight)
        #expect(!box.ringed)

        box.open(true, windowWidth: 1440)
        box.open(false, windowWidth: 1440)
        let back = Self.frame(of: box.frameView, in: window)
        #expect(back == resting)
        #expect(!box.ringed)
    }

    @Test func thePanelHangsFromTheBoxNotTheTextInsideIt() {
        let box = ToolbarSearchBox()
        #expect(box.field.frameView === box.frameView)
    }
}
