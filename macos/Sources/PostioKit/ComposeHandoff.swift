import AppKit
import Foundation
import PostioFFI

/// Opening a draft in the editor somebody chose (#1288).
///
/// Canvas 26 labels the hand-off `Open in $EDITOR`. An application launched
/// from Finder has **no shell environment**, so `$EDITOR` is usually simply
/// absent here and that label would name nothing. `[compose] editor` in
/// `config.toml` is the setting instead, and this is the AppKit half of
/// acting on it.
///
/// # What is decided here and what is not
///
/// Only one question is a fact about this Mac: whether an application by that
/// name exists. `NSWorkspace` answers it. **Everything that follows** — that
/// a name which is not an application is a command, that a command needs a
/// terminal, and what to say about it — is `postio_ui::handoff`, so GTK
/// reaches the same conclusion about the same name rather than a similar one.
public enum ComposeHandoff {
    /// The editor `[compose] editor` names, read when it is needed.
    ///
    /// From the file rather than from a cached copy: the compose window may
    /// have been opened before the setting was, and re-reading one small
    /// TOML file when a window appears is cheaper than being wrong about
    /// where somebody's draft is going.
    public static func configuredEditor() -> String {
        guard let path = try? settingsPath() else { return "" }
        return settingsComposing(text: settingsLoad(path: path))?.editor ?? ""
    }

    /// Whether this Mac has an application by that name.
    public static func isApplication(_ name: String) -> Bool {
        applicationURL(name) != nil
    }

    /// The application `name` refers to, in the three spellings somebody
    /// might reasonably write.
    ///
    /// A **path** first, which is the escape hatch for an application
    /// somewhere unusual and the only unambiguous form. Then the name as it
    /// appears in the Dock (`BBEdit`, `Visual Studio Code`), looked for in
    /// the applications directories — `urlForApplication(withName:)` is gone
    /// from the SDK, so the search is written out rather than borrowed.
    /// Then a **bundle identifier** (`com.apple.TextEdit`), which is what
    /// Launch Services actually keys on and the form that survives an
    /// application being renamed.
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

    /// Where applications live on this Mac, user's own first.
    private static func applicationDirectories() -> [URL] {
        var directories: [URL] = []
        for domain: FileManager.SearchPathDomainMask in [.userDomainMask, .localDomainMask, .systemDomainMask] {
            directories += FileManager.default.urls(for: .applicationDirectory, in: domain)
        }
        // `/System/Applications/Utilities` and the like: one level of nesting
        // is how Apple ships several of its own, and a person naming one is
        // naming an application like any other.
        return directories + directories.map { $0.appendingPathComponent("Utilities") }
    }

    /// Open `file` per the configured editor. `nil` when it opened; a
    /// sentence for the compose window's status line when it did not.
    ///
    /// Asynchronous in the `Application` case because
    /// `openApplication(at:configuration:)` is, and its completion is the
    /// only thing that knows whether the application actually started —
    /// reporting success before that would be reporting a guess.
    @MainActor
    public static func open(_ file: URL, using configured: String) async -> String? {
        switch settingsHandoffTarget(
            configured: configured, isApplication: isApplication(configured)
        ) {
        case .platformDefault:
            // POSTIO-CONSENT: only from the compose window's hand-off button,
            // pressed for one draft. The URL is a `file:` URL inside Postio's
            // own private directory -- nothing leaves this machine and no
            // network is touched. What is being asked for is a local editor.
            return NSWorkspace.shared.open(file)
                ? nil
                : "Nothing on this Mac opened that file, so the draft is still here."
        case .needsTerminal(_, let advice):
            return advice
        case .application(let name):
            guard let application = applicationURL(name) else {
                // Between the check above and here somebody could have moved
                // it; more usefully, this is what a typo looks like.
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
        // The point of the hand-off is to type in the other editor, so it
        // comes forward. Postio's own window is about to go read-only.
        configuration.activates = true
        return configuration
    }
}
