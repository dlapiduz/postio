import Foundation
import UniformTypeIdentifiers

/// What a file says it is (#1269).
///
/// A platform service, and the only part of attaching a file that is one:
/// macOS keeps a type database and answers from the extension and the file's
/// own content together, the way `gio` does on freedesktop. Everything else
/// about an attachment — the size guard, the blob write, the row on the draft
/// — is `postio_session::attaching`'s and happens once.
public enum MimeType {
    /// The MIME type of `file`, or `application/octet-stream`.
    ///
    /// Falling back rather than failing is deliberate: "some bytes" is better
    /// than refusing to attach a file because nothing recognised its type,
    /// and a recipient's client will sniff it again anyway.
    public static func of(_ file: URL) -> String {
        guard
            let type = UTType(filenameExtension: file.pathExtension),
            let mime = type.preferredMIMEType
        else {
            return fallback
        }
        return mime
    }

    /// What an unrecognised file is called.
    public static let fallback = "application/octet-stream"
}
