import Foundation

/// Parsed here rather than in the app target so it can be asserted: URL handling arrives through an
/// `NSApplication` callback a test cannot raise.
public struct Mailto: Equatable, Sendable {
    public let to: [String]
    public let cc: [String]
    public let bcc: [String]
    public let subject: String?
    public let body: String?

    /// A bare `mailto:` with no address is valid and means "compose".
    public init?(_ url: URL) {
        guard url.scheme?.lowercased() == "mailto" else { return nil }

        // `URLComponents` puts everything between `mailto:` and `?` in `path`, unparsed; decoded by
        // hand because `url.path` normalises in ways an address should not be.
        let components = URLComponents(url: url, resolvingAgainstBaseURL: false)
        let path = components?.path ?? ""
        let items = components?.queryItems ?? []

        func field(_ name: String) -> String? {
            items.first { $0.name.lowercased() == name }?.value
        }
        func addresses(_ raw: String?) -> [String] {
            (raw ?? "")
                .split(separator: ",")
                .map { $0.trimmingCharacters(in: .whitespaces) }
                .filter { !$0.isEmpty }
        }

        to = addresses(path) + addresses(field("to"))
        cc = addresses(field("cc"))
        bcc = addresses(field("bcc"))
        subject = field("subject")
        body = field("body")
        // Every other header field is dropped: RFC 6068 warns against honouring them blindly, and
        // `from` would let a link choose which of your accounts it appears to come from.
    }
}
