import AppKit
import PostioFFI
import Testing
@testable import PostioKit

/// What the window remembers, and the two ways remembering goes wrong.
///
/// Size, position and split widths are `NSWindow` frame autosave and
/// `SceneStorage` — Apple's, tested by Apple. What is Postio's is the pair of
/// decisions underneath: which folder to reopen when the remembered one is
/// gone, and where to put a window saved on a display that is no longer here.
/// Both fail silently and both are pure functions.
@Suite("Window state")
struct WindowStateTests {
    private func mailbox(
        _ id: Int64,
        _ role: MailboxRoleFfi,
        _ name: String,
        selectable: Bool = true
    ) -> MailboxFfi {
        MailboxFfi(
            id: id,
            account: 1,
            parent: nil,
            name: name,
            role: role,
            unread: 0,
            total: 0,
            selectable: selectable
        )
    }

    private var folders: [MailboxFfi] {
        [mailbox(1, .inbox, "Inbox"), mailbox(2, .archive, "Archive"), mailbox(3, .regular, "Work")]
    }

    // MARK: - Which folder reopens

    @Test("the folder that was open reopens")
    func remembersTheFolder() {
        #expect(WindowState.folderToOpen(remembered: 3, among: folders) == 3)
    }

    @Test("a folder that no longer exists falls back to the inbox")
    func fallsBackToInbox() {
        // A remembered mailbox can be renamed or unsubscribed between
        // launches. Opening the inbox is right; an empty list under a folder
        // name that is gone is not.
        #expect(WindowState.folderToOpen(remembered: 99, among: folders) == 1)
    }

    @Test("nothing remembered opens the inbox")
    func firstLaunchOpensInbox() {
        #expect(WindowState.folderToOpen(remembered: nil, among: folders) == 1)
    }

    @Test("an account with no inbox opens its first folder rather than nothing")
    func noInbox() {
        let odd = [mailbox(7, .regular, "Work"), mailbox(8, .archive, "Archive")]
        #expect(WindowState.folderToOpen(remembered: 99, among: odd) == 7)
    }

    @Test("a container folder is never reopened, even if it was remembered")
    func refusesANoselectFolder() {
        // A `\Noselect` folder is a node in the hierarchy, not a mailbox --
        // opening it shows nothing. It can also *become* one between launches,
        // which is how a remembered id ends up pointing at a container.
        let tree = [
            mailbox(1, .inbox, "Inbox"),
            mailbox(4, .regular, "Projects", selectable: false),
        ]
        #expect(WindowState.folderToOpen(remembered: 4, among: tree) == 1)
    }

    @Test("the fallback skips containers too")
    func fallbackSkipsContainers() {
        let tree = [
            mailbox(4, .regular, "Projects", selectable: false),
            mailbox(5, .regular, "Work"),
        ]
        #expect(WindowState.folderToOpen(remembered: 99, among: tree) == 5)
    }

    @Test("a store with no folders at all opens nothing")
    func noFolders() {
        // The first-run state, before any sync. Nothing to open is a real
        // answer here, not a failure to find one.
        #expect(WindowState.folderToOpen(remembered: 3, among: []) == nil)
    }

    // MARK: - Where the window opens

    private let screen = CGRect(x: 0, y: 0, width: 1920, height: 1080)

    @Test("a frame already on screen is left exactly as it was")
    func onScreenIsUntouched() {
        let saved = CGRect(x: 100, y: 100, width: 1100, height: 700)
        #expect(WindowState.visibleFrame(for: saved, on: [screen]) == saved)
    }

    @Test("a frame from a display that is gone comes back on screen")
    func offScreenIsRecovered() {
        // The case `NSWindow`'s autosave does not handle: a window saved on a
        // second display that is no longer attached opens at coordinates
        // nothing can reach, and the application looks like it failed to
        // launch.
        let saved = CGRect(x: 3000, y: 200, width: 1100, height: 700)
        let recovered = WindowState.visibleFrame(for: saved, on: [screen])
        #expect(screen.contains(recovered), "the window is somewhere reachable")
        #expect(recovered.size == saved.size, "its size is still what was saved")
    }

    @Test("a window mostly off the edge is pulled back")
    func partiallyOffScreen() {
        // Enough of it showing to grab is the standard, and a sliver is not
        // enough. Dragging a window nearly off the right edge and quitting is
        // an ordinary thing to do.
        let saved = CGRect(x: 1890, y: 100, width: 1100, height: 700)
        let recovered = WindowState.visibleFrame(for: saved, on: [screen])
        #expect(screen.contains(recovered))
    }

    @Test("a window larger than the screen is shrunk to fit rather than moved off it")
    func tooLarge() {
        // Saved on a large display, reopened on a laptop. Moving it cannot
        // help; only resizing can, and it has to stay usable rather than
        // become a strip.
        let saved = CGRect(x: 0, y: 0, width: 3000, height: 2000)
        let recovered = WindowState.visibleFrame(for: saved, on: [screen])
        #expect(screen.contains(recovered))
        #expect(recovered.width <= screen.width)
        #expect(recovered.height <= screen.height)
    }

    @Test("with no screens at all the saved frame is returned untouched")
    func noScreens() {
        // There is nothing better to answer, and inventing a frame would be
        // worse than letting the window server place it.
        let saved = CGRect(x: 3000, y: 200, width: 1100, height: 700)
        #expect(WindowState.visibleFrame(for: saved, on: []) == saved)
    }

    @Test("the second display is a fine place to open")
    func multipleDisplays() {
        // Still attached, so the saved frame is not stale and moving it would
        // be the bug rather than the fix.
        let second = CGRect(x: 1920, y: 0, width: 1920, height: 1080)
        let saved = CGRect(x: 2000, y: 100, width: 1100, height: 700)
        #expect(WindowState.visibleFrame(for: saved, on: [screen, second]) == saved)
    }
}
