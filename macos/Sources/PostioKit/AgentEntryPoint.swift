import Foundation

/// Where an agent's client should launch `postio-mcp` on this Mac (#1832).
///
/// The bundle script puts it beside the app's own executable, so the path is
/// derived from where the running app is rather than from `/Applications`:
/// a copy kept elsewhere shows its own path. `nil` when the app is not a
/// bundle (`swift run`), where there is no entry point to point at.
public enum AgentEntryPoint {
    public static func path(executable: URL?) -> String? {
        guard let executable else { return nil }
        let candidate = executable.deletingLastPathComponent()
            .appendingPathComponent("postio-mcp")
        guard candidate.path.contains(".app/Contents/MacOS/") else { return nil }
        return candidate.path
    }
}
