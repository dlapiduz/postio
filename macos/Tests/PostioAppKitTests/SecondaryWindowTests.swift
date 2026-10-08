import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// The secondary windows over Focus's list (specs/009-focus-macos T065,
/// contracts/mac-surfaces.md): one at a time (M4), centred on the main
/// window and sized from the engine's geometry (M1), and every way of
/// closing one reported once, so the controller's stack and the screen
/// never disagree about what is open.
///
/// No window is put on screen: `present` is the one step that orders a
/// window in, and these replace it, so the suite needs no window server
/// for anything but the windows' own objects.
@MainActor
struct SecondaryWindowTests {
    /// A main window, never shown.
    static func main(_ frame: NSRect = NSRect(x: 100, y: 100, width: 1440, height: 900)) -> NSWindow {
        let window = NSWindow(
            contentRect: frame, styleMask: [.titled, .closable, .resizable],
            backing: .buffered, defer: true)
        window.isReleasedWhenClosed = false
        window.setFrame(frame, display: false)
        return window
    }

    /// What was reported closed, in order.
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
        // 720 wide (M1 at 1440), the main window's height less 80, and
        // centred both ways: x 100 + (1440 - 720) / 2, y 100 + 40.
        #expect(frame == NSRect(x: 460, y: 140, width: 720, height: 820))
        #expect(controller.kind == .message)
        #expect(controller.window?.title == "Harbor", "the Window menu and VoiceOver name it")
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
        // M4: one secondary window at a time.
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
        // j and k never resize the window, and a size the person chose
        // survives the next message.
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
        // FocusCloseSurface (Esc through Back): close, then say so.
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
        // The width is the engine's (M1 keeps 640 below 1024); the height
        // follows the main window, and never goes negative.
        let (controller, _) = Self.controller()
        let main = Self.main(NSRect(x: 0, y: 0, width: 800, height: 60))
        controller.show(.message, content: NSView(), width: 640, title: "", over: main)
        let frame = controller.window?.frame
        #expect(frame?.width == 640)
        #expect((frame?.height ?? 0) >= SecondaryWindowController.minimumHeight)
    }

    @Test func aToolbarTheContentPutsOnDoesNotGrowTheWindow() {
        // The message window's title area is a unified toolbar, and AppKit
        // keeps the content's size when one is added -- so the window grew
        // past the main window by the toolbar's height.
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
