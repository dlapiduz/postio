import AppKit
import PostioKit

/// What Postio does when the system hands it a `mailto:` link.
///
/// Declaring the scheme without handling it would be the worst of the
/// available options: the user sets Postio as their mail client, clicks a
/// link, and nothing happens anywhere. So until compose exists this says so,
/// out loud, naming the address it was asked to write to — the issue's own
/// standard, *fails visibly rather than silently*.
///
/// The parsing is `Mailto` in PostioKit, where it is tested. This is only the
/// part that needs an application to exist.
@MainActor
final class URLHandler: NSObject, NSApplicationDelegate {
    func application(_: NSApplication, open urls: [URL]) {
        for url in urls {
            guard let mailto = Mailto(url) else { continue }
            announce(mailto)
        }
    }

    /// Say what was asked for, and that Postio cannot do it yet.
    ///
    /// A sheet rather than a notification: this is the direct result of
    /// something the user just clicked, and an answer to a click belongs in
    /// front of them rather than in Notification Centre.
    private func announce(_ mailto: Mailto) {
        let alert = NSAlert()
        alert.messageText = "Postio cannot compose yet"
        // The recipient, because it tells the user the link was read correctly
        // and it is their own address book, not content Postio went and
        // fetched. Subject and body are deliberately not shown: they can carry
        // anything the linking page chose, and this dialog can appear over a
        // locked screen's login window.
        alert.informativeText = switch mailto.to.first {
        case let address?: "This link asks to write to \(address). Writing mail is not built yet."
        case nil: "This link asks to write a new message. Writing mail is not built yet."
        }
        alert.alertStyle = .informational
        alert.addButton(withTitle: "OK")
        NSApp.activate(ignoringOtherApps: true)
        alert.runModal()
    }
}
