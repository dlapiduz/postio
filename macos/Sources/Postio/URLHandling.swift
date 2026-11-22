import AppKit
import PostioAppKit
import PostioKit

@MainActor
final class URLHandler: NSObject, NSApplicationDelegate {
    /// Installed from `Engine.init`, inside `App.init()` before SwiftUI builds anything, so
    /// SwiftUI's menu lands on top. The `async` hop puts this after the scene's first update
    /// (#1262).
    func applicationDidFinishLaunching(_: Notification) {
        switch DemoMode.appearance {
        case "dark": NSApp.appearance = NSAppearance(named: .darkAqua)
        case "light": NSApp.appearance = NSAppearance(named: .aqua)
        default: break
        }
        // In front: launched from a script it opens behind the terminal, and WebKit stops painting
        // a window it thinks is covered.
        if DemoMode.seed != nil { NSApp.activate(ignoringOtherApps: true) }
        AccentHue.follow()
        MenuBar.reassert()
        DispatchQueue.main.async { MenuBar.reassert() }
    }

    /// Quitting is the only right moment: `⌘W` and `⌘H` put the window away, and a mail client with
    /// no window is one collecting mail (`SessionLifetime`).
    var stop: (() -> Void)?

    func applicationWillTerminate(_: Notification) {
        stop?()
    }

    var write: ((Mailto) -> Bool)?

    /// A link that launched Postio can arrive before this is set; those wait and go as soon as it
    /// is.
    var follow: ((PostioLink.Route) -> Void)? {
        didSet {
            guard let follow else { return }
            for route in early.take() { follow(route) }
        }
    }

    private var early = PostioLink.Waiting()

    func application(_: NSApplication, open urls: [URL]) {
        for url in urls {
            if let route = PostioLink.route(url) {
                NSApp.activate(ignoringOtherApps: true)
                if let follow { follow(route) } else { early.hold(route) }
                continue
            }
            guard let mailto = Mailto(url) else { continue }
            if write?(mailto) != true {
                announce(mailto)
            }
        }
    }

    /// A sheet rather than a notification: it answers a click, so it belongs in front of the user.
    private func announce(_ mailto: Mailto) {
        let alert = NSAlert()
        alert.messageText = "Postio could not open a composer"
        // Only the recipient is shown: subject and body can carry anything the linking page chose,
        // and this dialog can appear over a locked screen's login window.
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
