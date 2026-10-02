import Foundation

/// Writing a path the way a person writes it.
///
/// Two footers name a file — the composer's says where the draft is, the
/// settings window's says what it is writing — and both are read at a
/// glance. `/Users/diego` is nine characters of prefix telling somebody
/// something they already know, and in a footer that truncates, it is nine
/// characters taken from the part that identifies the file.
public enum PostioPath {
    /// `/Users/diego/Library/…` as `~/Library/…`.
    ///
    /// Anything outside the home directory is left exactly as it is: a path
    /// under `/var` or on another volume is unusual, and *that* is worth
    /// noticing rather than shortening.
    public static func abbreviated(_ path: String) -> String {
        let home = NSHomeDirectory()
        guard !home.isEmpty, path.hasPrefix(home) else { return path }
        return "~" + path.dropFirst(home.count)
    }
}
