import Foundation
import ImageIO
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

    /// What a picture's bytes say it is, or `nil` when they are not one
    /// (#1571).
    ///
    /// From the bytes rather than the name, for the reason `postio-gtk`'s
    /// composer gives: a `.png` that is really a JPEG would reach the
    /// recipient declared wrongly, and the declaration is all their client
    /// has to go on. `ImageIO`, not AppKit, so this goes to a phone as it is.
    public static func ofImage(_ bytes: Data) -> String? {
        guard !bytes.isEmpty,
              let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              let identifier = CGImageSourceGetType(source) as String?,
              let mime = UTType(identifier)?.preferredMIMEType
        else { return nil }
        return mime
    }
}
