import CoreGraphics
import Foundation
import ImageIO
import Testing
import UniformTypeIdentifiers

@testable import PostioKit

@Suite struct MimeTypeTests {
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
        #expect(MimeType.ofImage(Data("%PDF-1.7".utf8)) == nil)
        #expect(MimeType.ofImage(Data()) == nil)
    }
}
