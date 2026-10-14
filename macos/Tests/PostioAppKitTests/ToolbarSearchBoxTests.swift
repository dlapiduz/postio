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

    /// Whether this screen holds a window `width` wide. A CI runner's is
    /// 1024: AppKit squeezes a wider titled window onto it, and one kept
    /// wider anyway hangs off the screen, where the toolbar lays out for a
    /// width nobody sees. So the cases at the design's 1440 run only where
    /// a 1440 window fits, and 1024 runs everywhere.
    static func screenHolds(_ width: CGFloat) -> Bool {
        (NSScreen.main?.visibleFrame.width ?? 0) >= width
    }

    /// A window `width` wide with the inbox's toolbar shape: Compose, a
    /// flexible space, the field.
    static func window(width: CGFloat) -> (NSWindow, Delegate) {
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 40, width: width, height: 700),
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
        #expect(window.frame.width == width, "as wide as asked: the screen holds it")
        return (window, delegate)
    }

    /// The box's frame on screen, after the toolbar has laid it out.
    static func frame(of box: NSView, in window: NSWindow) -> CGRect {
        window.contentView?.superview?.layoutSubtreeIfNeeded()
        return window.convertToScreen(box.convert(box.bounds, to: nil))
    }

    /// At rest the field says the short placeholder (McList.dc.html);
    /// opened, the long one (screen 01).
    @Test func atRestTheFieldSaysLessThanOpened() {
        let box = ToolbarSearchBox()
        box.restingPlaceholder = "Search mail or run a command"
        box.placeholder = "Search mail, people and files, or type > for commands"
        #expect(box.field.placeholderAttributedString?.string == "Search mail or run a command")
        box.open(true, windowWidth: 1440)
        #expect(
            box.field.placeholderAttributedString?.string
                == "Search mail, people and files, or type > for commands")
        box.open(false, windowWidth: 1440)
        #expect(box.field.placeholderAttributedString?.string == "Search mail or run a command")
    }

    @Test func openedTheFieldGrowsLeftwardWithItsRightEdge12FromTheWindows() {
        Self.opens(width: 1024)
    }

    @Test(.enabled("the screen is narrower than the design's window") {
        await ToolbarSearchBoxTests.screenHolds(1440)
    })
    func openedAt1440TheFieldIs860() {
        Self.opens(width: 1440)
    }

    static func opens(width: CGFloat) {
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
        let (window, delegate) = Self.window(width: 1024)
        defer { window.close() }
        let box = delegate.box
        let resting = Self.frame(of: box.frameView, in: window)
        #expect(resting.width == CommandBarGeometry.restingWidth)
        #expect(resting.height == ToolbarSearchBox.restingHeight)
        #expect(!box.ringed)

        box.open(true, windowWidth: 1024)
        box.open(false, windowWidth: 1024)
        let back = Self.frame(of: box.frameView, in: window)
        #expect(back == resting)
        #expect(!box.ringed)
    }

    @Test func thePanelHangsFromTheBoxNotTheTextInsideIt() {
        let box = ToolbarSearchBox()
        #expect(box.field.frameView === box.frameView)
    }
}
