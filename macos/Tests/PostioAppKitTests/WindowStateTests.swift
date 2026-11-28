import AppKit
import PostioFFI
import Testing
@testable import PostioAppKit
@testable import PostioKit

@Suite("Window state")
struct WindowStateTests {
    private func mailbox(
        _ id: Int64,
        _ role: MailboxRoleFfi,
        _ name: String,
        selectable: Bool = true,
        special: Bool = true
    ) -> MailboxFfi {
        MailboxFfi(
            id: id,
            account: 1,
            parent: nil,
            name: name,
            role: role,
            unread: 0,
            total: 0,
            selectable: selectable,
            lastSyncedAt: nil,
            special: special,
            flagged: 0,
            snoozed: 0
        )
    }

    private var folders: [MailboxFfi] {
        [mailbox(1, .inbox, "Inbox"), mailbox(2, .archive, "Archive"), mailbox(3, .regular, "Work")]
    }

    @Test("the folder that was open reopens")
    func remembersTheFolder() {
        #expect(WindowState.folderToOpen(remembered: 3, among: folders) == 3)
    }

    @Test("a folder that no longer exists falls back to the inbox")
    func fallsBackToInbox() {
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
        #expect(WindowState.folderToOpen(remembered: 3, among: []) == nil)
    }

    private let screen = CGRect(x: 0, y: 0, width: 1920, height: 1080)

    @Test("a frame already on screen is left exactly as it was")
    func onScreenIsUntouched() {
        let saved = CGRect(x: 100, y: 100, width: 1100, height: 700)
        #expect(WindowState.visibleFrame(for: saved, on: [screen]) == saved)
    }

    @Test("a frame from a display that is gone comes back on screen")
    func offScreenIsRecovered() {
        let saved = CGRect(x: 3000, y: 200, width: 1100, height: 700)
        let recovered = WindowState.visibleFrame(for: saved, on: [screen])
        #expect(screen.contains(recovered), "the window is somewhere reachable")
        #expect(recovered.size == saved.size, "its size is still what was saved")
    }

    @Test("a window mostly off the edge is pulled back")
    func partiallyOffScreen() {
        let saved = CGRect(x: 1890, y: 100, width: 1100, height: 700)
        let recovered = WindowState.visibleFrame(for: saved, on: [screen])
        #expect(screen.contains(recovered))
    }

    @Test("a window larger than the screen is shrunk to fit rather than moved off it")
    func tooLarge() {
        let saved = CGRect(x: 0, y: 0, width: 3000, height: 2000)
        let recovered = WindowState.visibleFrame(for: saved, on: [screen])
        #expect(screen.contains(recovered))
        #expect(recovered.width <= screen.width)
        #expect(recovered.height <= screen.height)
    }

    @Test("with no screens at all the saved frame is returned untouched")
    func noScreens() {
        let saved = CGRect(x: 3000, y: 200, width: 1100, height: 700)
        #expect(WindowState.visibleFrame(for: saved, on: []) == saved)
    }

    @Test("the second display is a fine place to open")
    func multipleDisplays() {
        let second = CGRect(x: 1920, y: 0, width: 1920, height: 1080)
        let saved = CGRect(x: 2000, y: 100, width: 1100, height: 700)
        #expect(WindowState.visibleFrame(for: saved, on: [screen, second]) == saved)
    }
}
