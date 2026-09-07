import AppKit
import PostioKit

/// What Postio does when the system hands it a `mailto:` link.
///
/// It opens a composer on it. Until #1272 there was nothing to open, and
/// this said so out loud rather than doing nothing — the right answer then,
/// and a stale one the moment writing mail started working.
///
/// The parsing is `Mailto` in PostioKit, where it is tested; assembling the
/// draft is the boundary's, so GTK gets the same behaviour from the same
/// code. This is only the part that needs an application to exist.
@MainActor
final class URLHandler: NSObject, NSApplicationDelegate {
    /// Put Postio's menu bar back, after SwiftUI has finished building its
    /// own.
    ///
    /// The bar is installed from `Engine.init`, which runs inside
    /// `App.init()` — before SwiftUI builds anything, so SwiftUI's own menu
    /// lands on top of it. This is the first moment the application is
    /// running and SwiftUI has had its turn; the `async` hop is what puts it
    /// *after* the scene's first update rather than in the middle of it
    /// (#1262).
    func applicationDidFinishLaunching(_: Notification) {
        MenuBar.reassert()
        DispatchQueue.main.async { MenuBar.reassert() }
    }

    /// Where a `mailto:` goes. Set by the application, which owns the
    /// session and the compose windows.
    var write: ((Mailto) -> Bool)?

    func application(_: NSApplication, open urls: [URL]) {
        for url in urls {
            guard let mailto = Mailto(url) else { continue }
            // A composer, when there is one to open. `false` means there was
            // not — no account to write from, or no session yet — and that
            // still has to be said rather than swallowed.
            if write?(mailto) != true {
                announce(mailto)
            }
        }
    }

    /// Say what was asked for, when no composer could be opened for it.
    ///
    /// A sheet rather than a notification: this is the direct result of
    /// something the user just clicked, and an answer to a click belongs in
    /// front of them rather than in Notification Centre.
    private func announce(_ mailto: Mailto) {
        let alert = NSAlert()
        alert.messageText = "Postio could not open a composer"
        // The recipient, because it tells the user the link was read correctly
        // and it is their own address book, not content Postio went and
        // fetched. Subject and body are deliberately not shown: they can carry
        // anything the linking page chose, and this dialog can appear over a
        // locked screen's login window.
        alert.informativeText = switch mailto.to.first {
        case let address?:
            "This link asks to write to \(address). Postio has no account to "
                + "write from — add one in Settings."
        case nil:
            "This link asks to write a new message. Postio has no account to "
                + "write from — add one in Settings."
        }
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }
}
