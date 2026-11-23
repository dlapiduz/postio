import AppKit
import PostioFFI
import Testing

@testable import PostioAppKit
@testable import PostioKit

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
        // A panel AppKit put up on its own has no registry bindings; answering `.main` keeps the
        // old behaviour.
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
    @Test func aComposeWindowSaysWhichDraftItIsWriting() {
        // Several composers can be open: a verb must reach the one with the keyboard, or Send sends
        // another.
        let tracker = KeyWindowTracker()
        let window = NSWindow(
            contentRect: .init(x: 0, y: 0, width: 10, height: 10),
            styleMask: [.titled], backing: .buffered, defer: true
        )
        KeyWindowTracker.tag(window, as: .compose, draft: 7)

        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification, object: window
        )
        #expect(tracker.current == .compose, "the role survives the draft riding on the tag")
        #expect(tracker.currentDraft == 7)

        NotificationCenter.default.post(
            name: NSWindow.didResignKeyNotification, object: window
        )
        #expect(tracker.currentDraft == nil, "no compose window in front, no draft")
    }

    @Test func aWindowWithNoDraftSaysSo() {
        let tracker = KeyWindowTracker()
        NotificationCenter.default.post(
            name: NSWindow.didBecomeKeyNotification, object: window(.main)
        )
        #expect(tracker.currentDraft == nil)
    }

}
