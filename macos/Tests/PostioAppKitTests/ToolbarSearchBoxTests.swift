import AppKit
import Testing

@testable import PostioAppKit
@testable import PostioKit

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

    static func screenHolds(_ width: CGFloat) -> Bool {
        (NSScreen.main?.visibleFrame.width ?? 0) >= width
    }

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

    static func frame(of box: NSView, in window: NSWindow) -> CGRect {
        window.contentView?.superview?.layoutSubtreeIfNeeded()
        return window.convertToScreen(box.convert(box.bounds, to: nil))
    }

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
