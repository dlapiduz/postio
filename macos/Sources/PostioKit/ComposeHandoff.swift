import Foundation
import PostioFFI

public enum ComposeHandoff {
    /// Read from the file, not a cache: the window may have been opened before the setting was.
    public static func configuredEditor() -> String {
        guard let path = try? settingsPath() else { return "" }
        return settingsComposing(text: settingsLoad(path: path))?.editor ?? ""
    }
}
