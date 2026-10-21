import Observation

/// A request, made by a command, for the settings window to open on one
/// section (`v y` opens Contacts). Held outside the window for the reason
/// `SettingsAccounts` is: a command cannot reach a view's `@State`, and it may
/// ask before the window exists. The window takes the request when it appears
/// and whenever the token moves.
@MainActor
@Observable
public final class SettingsSectionRequest {
    /// Moves on every request, so asking for the same section twice in a row
    /// still moves the window.
    public private(set) var token = 0
    @ObservationIgnored private var section: String?

    public init() {}

    /// Ask for the section with this key.
    public func ask(_ key: String) {
        section = key
        token += 1
    }

    /// The section asked for, once.
    public func take() -> String? {
        defer { section = nil }
        return section
    }
}
