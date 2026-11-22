import Observation

/// Held outside the window for the reason `SettingsAccounts` is: a command cannot reach `@State`,
/// and may ask before the window exists.
@MainActor
@Observable
public final class SettingsSectionRequest {
    /// Moves on every request, so asking for the same section twice in a row
    /// still moves the window.
    public private(set) var token = 0
    @ObservationIgnored private var section: String?

    public init() {}

    public func ask(_ key: String) {
        section = key
        token += 1
    }

    public func take() -> String? {
        defer { section = nil }
        return section
    }
}
