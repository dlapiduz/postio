import Foundation

/// Footers that name a file abbreviate the home directory, which the reader already knows and
/// truncation would otherwise take from the identifying part.
public enum PostioPath {
    /// Anything outside the home directory is left as is: an unusual path is worth noticing.
    public static func abbreviated(_ path: String) -> String {
        let home = NSHomeDirectory()
        guard !home.isEmpty, path.hasPrefix(home) else { return path }
        return "~" + path.dropFirst(home.count)
    }
}
