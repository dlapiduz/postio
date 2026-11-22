import Foundation
import ImageIO
import UniformTypeIdentifiers

/// A platform service: macOS answers from the extension and the content together, as `gio` does.
/// The size guard, blob write and draft row are `postio_session::attaching`'s.
public enum MimeType {
    /// Falls back to `application/octet-stream` rather than refusing to attach; a recipient's
    /// client sniffs it again.
    public static func of(_ file: URL) -> String {
        guard
            let type = UTType(filenameExtension: file.pathExtension),
            let mime = type.preferredMIMEType
        else {
            return fallback
        }
        return mime
    }

    public static let fallback = "application/octet-stream"

    /// `nil` when the bytes are not an image. Read from the bytes, not the name: a `.png` that is a
    /// JPEG would reach the recipient mis-declared. `ImageIO`, not AppKit, so it goes to a phone as
    /// it is.
    public static func ofImage(_ bytes: Data) -> String? {
        guard !bytes.isEmpty,
              let source = CGImageSourceCreateWithData(bytes as CFData, nil),
              let identifier = CGImageSourceGetType(source) as String?,
              let mime = UTType(identifier)?.preferredMIMEType
        else { return nil }
        return mime
    }
}
