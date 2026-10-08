import Foundation
import PostioFFI

/// Where a draft goes when `[compose] editor` names another program. The
/// AppKit half (finding and launching the application) is in PostioAppKit.
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
}
