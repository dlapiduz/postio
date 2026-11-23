import AppKit
import PostioKit
import Foundation
import PostioFFI

/// An app launched from Finder has no shell environment, so `$EDITOR` is usually absent;
/// `[compose] editor` in `config.toml` is the setting. Only whether an application by that name
/// exists is decided here; the rest is `postio_ui::handoff`, shared with GTK.
extension ComposeHandoff {
    /// Three answers: `vim` is real and has no window, `vum` is neither, and calling the second
    /// "runs in a terminal" is wrong (#1297).
    public static func found(_ name: String) -> FoundEditorFfi {
        if applicationURL(name) != nil { return .application }
        return onPath(name) ? .terminalProgram : .nothing
    }

    public static func isApplication(_ name: String) -> Bool {
        applicationURL(name) != nil
    }

    private static func onPath(_ name: String) -> Bool {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty, !trimmed.contains("/") else { return false }
        let path = ProcessInfo.processInfo.environment["PATH"]
            // Finder-launched apps inherit no shell environment, so `PATH` is often absent.
            ?? "/usr/bin:/bin:/usr/local/bin:/opt/homebrew/bin"
        return path.split(separator: ":").contains { directory in
            FileManager.default.isExecutableFile(
                atPath: URL(fileURLWithPath: String(directory))
                    .appendingPathComponent(trimmed).path
            )
        }
    }

    /// A path first, then the Dock name searched in the applications directories
    /// (`urlForApplication(withName:)` is gone from the SDK), then a bundle identifier, which
    /// survives renames.
    static func applicationURL(_ name: String) -> URL? {
        let trimmed = name.trimmingCharacters(in: .whitespaces)
        guard !trimmed.isEmpty else { return nil }

        if trimmed.hasPrefix("/") || trimmed.hasPrefix("~") {
            let path = (trimmed as NSString).expandingTildeInPath
            return FileManager.default.fileExists(atPath: path)
                ? URL(fileURLWithPath: path) : nil
        }

        let bundle = trimmed.hasSuffix(".app") ? trimmed : "\(trimmed).app"
        for directory in applicationDirectories() {
            let candidate = directory.appendingPathComponent(bundle)
            if FileManager.default.fileExists(atPath: candidate.path) {
                return candidate
            }
        }
        return NSWorkspace.shared.urlForApplication(withBundleIdentifier: trimmed)
    }

    private static func applicationDirectories() -> [URL] {
        var directories: [URL] = []
        for domain: FileManager.SearchPathDomainMask in [.userDomainMask, .localDomainMask, .systemDomainMask] {
            directories += FileManager.default.urls(for: .applicationDirectory, in: domain)
        }
        return directories + directories.map { $0.appendingPathComponent("Utilities") }
    }

    /// Asynchronous in the `Application` case: only the completion of
    /// `openApplication(at:configuration:)` knows whether it started.
    @MainActor
    public static func open(_ file: URL, using configured: String) async -> String? {
        switch settingsHandoffTarget(configured: configured, found: found(configured)) {
        case .platformDefault:
            // POSTIO-CONSENT: only from the compose window's hand-off button,
            // pressed for one draft. The URL is a `file:` URL inside Postio's
            // own private directory -- nothing leaves this machine and no
            // network is touched. What is being asked for is a local editor.
            return NSWorkspace.shared.open(file)
                ? nil
                : "Nothing on this Mac opened that file, so the draft is still here."
        case .needsTerminal(_, let advice), .missing(_, let advice):
            return advice
        case .application(let name):
            guard let application = applicationURL(name) else {
                return "This Mac has no application called \(name), so the draft is still here."
            }
            do {
                // POSTIO-CONSENT: as above, and narrower -- the application
                // is the one named in `[compose] editor`, and it is handed
                // one local file the user chose to hand it.
                try await NSWorkspace.shared.openApplication(
                    at: application,
                    configuration: openInFront(file)
                )
                return nil
            } catch {
                return "\(name) would not open the draft: \(error.localizedDescription)"
            }
        }
    }

    private static func openInFront(_ file: URL) -> NSWorkspace.OpenConfiguration {
        let configuration = NSWorkspace.OpenConfiguration()
        configuration.arguments = [file.path]
        // The hand-off is to type in the other editor, so it comes forward.
        configuration.activates = true
        return configuration
    }
}
