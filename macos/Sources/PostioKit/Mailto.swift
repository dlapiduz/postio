import Foundation

/// A `mailto:` URL, as RFC 6068 defines it.
///
/// Declaring the scheme in `Info.plist` is what makes Postio eligible to be
/// the machine's mail client — the difference between being *a* mail client on
/// it and *the* one. This is what Postio does when something takes it up.
///
/// Parsed here rather than in the app target so it can be asserted: URL
/// handling is delivered through an `NSApplication` callback that a test
/// cannot raise, and the parsing is the half with decisions in it.
public struct Mailto: Equatable, Sendable {
    /// Recipients, from the path and from any `to` header field.
    public let to: [String]
    public let cc: [String]
    public let bcc: [String]
    public let subject: String?
    public let body: String?

    /// Reads `url`, or `nil` if it is not a `mailto:`.
    ///
    /// A bare `mailto:` with no address is valid and means "compose" — it is
    /// what "New Message" in another application sends, and a composer with an
    /// empty To: field is the right answer to it.
    public init?(_ url: URL) {
        guard url.scheme?.lowercased() == "mailto" else { return nil }

        // `URLComponents` puts everything after `mailto:` and before `?` in
        // `path`, unparsed. Percent-decoding it by hand rather than trusting
        // `url.path`, which normalises in ways an address should not be.
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

        // RFC 6068 allows `to` as a header field as well as in the path, and
        // some applications only ever write it that way.
        to = addresses(path) + addresses(field("to"))
        cc = addresses(field("cc"))
        bcc = addresses(field("bcc"))
        subject = field("subject")
        body = field("body")
        // Every other header field is dropped. RFC 6068 permits arbitrary ones
        // and warns against honouring them blindly; `from` in particular would
        // let a link choose which of your accounts it appears to come from.
    }
}
