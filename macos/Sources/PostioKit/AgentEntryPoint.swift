import Foundation

/// Derived from where the running app is, not `/Applications`, so a copy kept elsewhere shows its
/// own path. `nil` when the app is not a bundle (`swift run`).
public enum AgentEntryPoint {
    public static func path(executable: URL?) -> String? {
        guard let executable else { return nil }
        let candidate = executable.deletingLastPathComponent()
            .appendingPathComponent("postio-mcp")
        guard candidate.path.contains(".app/Contents/MacOS/") else { return nil }
        return candidate.path
    }
}
