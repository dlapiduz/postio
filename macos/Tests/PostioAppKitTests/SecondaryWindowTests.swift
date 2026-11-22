import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

@MainActor
struct SecondaryWindowTests {
    static func main(_ frame: NSRect = NSRect(x: 100, y: 100, width: 1440, height: 900)) -> NSWindow {
        let window = NSWindow(
            contentRect: frame, styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.setFrame(frame, display: false)
        return window
    }

    final class Closed { var kinds: [SurfaceKindFfi] = [] }

    static func controller() -> (SecondaryWindowController, Closed) {
        let closed = Closed()
        let controller = SecondaryWindowController(present: { _, _ in }) { kind in
            closed.kinds.append(kind)
        }
        return (controller, closed)
    }

    @Test func theWindowIsCentredOnTheMainWindowAtTheEnginesWidth() {
        let (controller, _) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "Harbor", over: main)
        let frame = try? #require(controller.window?.frame)
        #expect(frame == NSRect(x: 460, y: 140, width: 720, height: 820))
        #expect(controller.kind == .message)
        #expect(controller.window?.title == "Harbor", "the Window menu and VoiceOver name it")
    }

    @Test func aSurfaceShorterThanTheMessageWindowKeepsItsOwnHeightCentred() {
        let (controller, _) = Self.controller()
        let main = Self.main()
        controller.show(.capture, content: NSView(), width: 660, height: 600, title: "", over: main)
        #expect(controller.window?.frame == NSRect(x: 490, y: 250, width: 660, height: 600))

        let (tall, _) = Self.controller()
        tall.show(.capture, content: NSView(), width: 660, height: 2000, title: "", over: main)
        #expect(tall.window?.frame.height == 820)
    }

    @Test func aSurfaceSizedToItsContentGrowsDownwardAndStaysUnderTheMessageWindowsHeight() {
        let (controller, _) = Self.controller()
        let main = Self.main()
        controller.show(.capture, content: NSView(), width: 660, height: 600, title: "", over: main)
        let window = controller.window
        let chrome = (window?.frame.height ?? 0) - (window?.contentLayoutRect.height ?? 0)
        let top = window?.frame.maxY

        controller.fit(contentHeight: 420)
        #expect(window?.frame.height == 420 + chrome)
        #expect(window?.frame.maxY == top, "it grows and shrinks from its top")
        #expect(window?.frame.width == 660)

        controller.fit(contentHeight: 560)
        #expect(window?.frame.height == 560 + chrome)
        #expect(window?.frame.maxY == top)

        controller.fit(contentHeight: 5000)
        #expect(window?.frame.height == 820, "no taller than the message window")
        #expect((window?.frame.minY ?? 0) >= main.frame.minY, "and never off the main window")

        controller.close(.capture)
        controller.fit(contentHeight: 300)
        #expect(controller.window == nil, "fitting nothing open does nothing")
    }

    @Test func itIsAChildOfTheMainWindowAndNeverMinimisesToTheDock() {
        let (controller, _) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "", over: main)
        let window = controller.window
        #expect(window?.styleMask.contains(.titled) == true, "traffic lights")
        #expect(window?.styleMask.contains(.miniaturizable) == false)
        #expect(window?.isReleasedWhenClosed == false)
        #expect(controller.parent === main)
    }

    @Test func openingAnotherKindClosesTheFirst() {
        let (controller, closed) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "", over: main)
        let first = controller.window
        controller.show(.composer, content: NSView(), width: 640, title: "", over: main)
        #expect(closed.kinds == [.message], "the message window said it closed")
        #expect(controller.kind == .composer)
        #expect(controller.window !== first)
    }

    @Test func theSameKindReplacesItsContentAndKeepsItsFrame() {
        let (controller, closed) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "One", over: main)
        let window = controller.window
        let chosen = NSRect(x: 300, y: 200, width: 800, height: 700)
        window?.setFrame(chosen, display: false)
        let next = NSView()
        controller.show(.message, content: next, width: 656, title: "Two", over: main)
        #expect(controller.window === window)
        #expect(controller.window?.frame == chosen)
        #expect(controller.window?.contentView === next)
        #expect(controller.window?.title == "Two")
        #expect(closed.kinds.isEmpty)
    }

    @Test func closingTheWindowReportsItOnce() {
        let (controller, closed) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "", over: main)
        controller.window?.close()
        #expect(closed.kinds == [.message], "⌘W and the close button are reported")
        #expect(controller.window == nil && controller.kind == nil)
        controller.close(.message)
        #expect(closed.kinds == [.message], "and only once")
    }

    @Test func theControllerClosingItIsReportedToo() {
        let (controller, closed) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "", over: main)
        controller.close(.composer)
        #expect(closed.kinds.isEmpty, "a kind that is not open closes nothing")
        controller.close(.message)
        #expect(closed.kinds == [.message])
        #expect(controller.window == nil)
    }

    @Test func closingTheMainWindowClosesTheSecondaryOne() {
        let (controller, closed) = Self.controller()
        let main = Self.main()
        controller.show(.message, content: NSView(), width: 720, title: "", over: main)
        main.close()
        #expect(closed.kinds == [.message])
        #expect(controller.window == nil)
    }

    @Test func aNarrowMainWindowKeepsTheMinimumAndFitsTheHeight() {
        let (controller, _) = Self.controller()
        let main = Self.main(NSRect(x: 0, y: 0, width: 800, height: 60))
        controller.show(.message, content: NSView(), width: 640, title: "", over: main)
        let frame = controller.window?.frame
        #expect(frame?.width == 640)
        #expect((frame?.height ?? 0) >= SecondaryWindowController.minimumHeight)
    }

    @Test func aToolbarTheContentPutsOnDoesNotGrowTheWindow() {
        let (controller, _) = Self.controller()
        let main = Self.main()
        controller.show(
            .message, content: NSView(), width: 720, title: "", over: main,
            configure: { window in
                window.toolbarStyle = .unified
                window.toolbar = NSToolbar(identifier: "test")
            })
        #expect(controller.window?.frame == NSRect(x: 460, y: 140, width: 720, height: 820))
    }
}
