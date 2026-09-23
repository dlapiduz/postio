import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@testable import PostioKit

/// What a picture's bytes say it is (#1571).
///
/// From the bytes rather than the name, for the reason `postio-gtk`'s
/// composer gives: a `.png` that is really a JPEG would reach the recipient
/// declared wrongly, and the declaration is all their client has to go on.
@Suite struct MimeTypeTests {
    /// A one-pixel picture, encoded as `type`.
    private func picture(_ type: UTType) throws -> Data {
        let context = try #require(
            CGContext(
                data: nil, width: 1, height: 1, bitsPerComponent: 8, bytesPerRow: 4,
                space: CGColorSpaceCreateDeviceRGB(),
                bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue
            )
        )
        let image = try #require(context.makeImage())
        let out = NSMutableData()
        let destination = try #require(
            CGImageDestinationCreateWithData(out, type.identifier as CFString, 1, nil)
        )
        CGImageDestinationAddImage(destination, image, nil)
        try #require(CGImageDestinationFinalize(destination))
        return out as Data
    }

    @Test func aPictureIsNamedByWhatItIsNotWhatItIsCalled() throws {
        #expect(MimeType.ofImage(try picture(.png)) == "image/png")
        #expect(MimeType.ofImage(try picture(.jpeg)) == "image/jpeg")
    }

    @Test func bytesThatAreNotAPictureAreNone() {
        // `nil`, so the caller falls back to the name and the boundary
        // refuses a non-image in its own words.
        #expect(MimeType.ofImage(Data("%PDF-1.7".utf8)) == nil)
        #expect(MimeType.ofImage(Data()) == nil)
    }
}
