import AppKit
import PostioFFI
import Testing

@testable import PostioKit

/// The tracker that tells the key monitor which window it is listening to.
///
/// `KeyboardContextTests` assert the *decision*; this asserts the thing that
/// feeds it, because the decision being right is worth nothing if nobody ever
/// says "a compose window has the keyboard". That is the shape of this port's
/// three worst bugs — built, tested, never mounted — so the tracker is tested
/// against the notifications AppKit actually sends rather than against a
/// method call.
///
/// No window server needed: `NSWindow.didBecomeKeyNotification` is an
/// ordinary notification, and a window that was never ordered on screen still
/// carries an identifier.
@MainActor
@Suite struct KeyWindowTrackerTests {
    private func window(_ role: KeyWindow?) -> NSWindow {
        let window = NSWindow(
            contentRect: .init(x: 0, y: 0, width: 10, height: 10),
            styleMask: [.titled],
            backing: .buffered,
            defer: true
        )
        if let role { KeyWindowTracker.tag(window, as: role) }
        return window
    }

    @Test func aComposeWindowBecomingKeyIsNoticed() {
        let tracker = KeyWindowTracker()
        #expect(tracker.current == .main, "the main window is where it starts")

        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification,
            object: window(.compose)
        )
        #expect(tracker.current == .compose)
    }

    @Test func closingTheComposeWindowPutsTheKeyboardBackInTheMainWindow() {
        let tracker = KeyWindowTracker()
        let composing = window(.compose)

        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification, object: composing
        )
        #expect(tracker.current == .compose)

        NotificationCenter.default.post(
            name: NSWindow.didResignKeyNotification, object: composing
        )
        #expect(
            tracker.current == .main,
            "with no compose window in front, keys are the main window's again"
        )
    }

    @Test func anUntaggedWindowIsTheMainWindow() {
        // A panel AppKit put up on its own — an open dialog, a font panel.
        // Whatever it is, it is not a surface with registry bindings, and
        // answering `.main` keeps the behaviour that was there before.
        let tracker = KeyWindowTracker()
        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification, object: window(nil)
        )
        #expect(tracker.current == .main)
    }

    @Test func theSettingsWindowIsItsOwnRole() {
        let tracker = KeyWindowTracker()
        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification, object: window(.settings)
        )
        #expect(tracker.current == .settings)
    }

    /// The whole point, stated as the bug: a letter typed into a compose
    /// window must not resolve as a list verb.
    @Test func typingIntoAComposeWindowDoesNotReachTheListsVerbs() {
        let tracker = KeyWindowTracker()
        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification, object: window(.compose)
        )
        let context = KeyboardContext.resolving(keyWindow: tracker.current, mainWindow: .list)

        let archive = PostioRegistry.commands.first { $0.id == "archive" }
        #expect(archive != nil)
        #expect(
            archive?.contexts.contains(context) == false,
            "`a` in a compose window still resolves to archive"
        )
    }
}
