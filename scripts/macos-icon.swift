// Renders Postio's application icon from the SVG the GTK frontend already
// ships, at the sizes macOS asks for, and leaves an `.iconset` for `iconutil`.
//
// Generated rather than redrawn, the same rule the design tokens follow: a set
// of PNGs checked in beside the SVG is a copy that is correct on the day it is
// made. The mark lives once, in `crates/postio-gtk/data/icons/`.
//
// AppKit rather than a rasterizer from Homebrew, deliberately. `NSImage` reads
// this SVG as a vector representation, so every size is a real render rather
// than an upscale of the 128px PNG — and it is on every Mac, which a build
// dependency nobody has is not.

import AppKit

guard CommandLine.arguments.count == 3 else {
    FileHandle.standardError.write(Data("usage: macos-icon.swift <svg> <iconset dir>\n".utf8))
    exit(2)
}
let source = URL(fileURLWithPath: CommandLine.arguments[1])
let iconset = URL(fileURLWithPath: CommandLine.arguments[2])

guard let artwork = NSImage(contentsOf: source) else {
    FileHandle.standardError.write(Data("could not read \(source.path)\n".utf8))
    exit(1)
}

try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

/// The names `iconutil` expects. Each logical size twice: the 1x file and the
/// 2x file of the size below it, which is what makes one `.icns` serve both a
/// Retina Dock and a 16px Finder list.
let sizes = [16, 32, 128, 256, 512]

func render(_ pixels: Int, to file: URL) throws {
    guard let rep = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: pixels,
        pixelsHigh: pixels,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ) else { throw CocoaError(.fileWriteUnknown) }

    // The representation's own pixel grid, so the vector is rasterised at this
    // size rather than drawn once and resampled.
    rep.size = NSSize(width: pixels, height: pixels)
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    artwork.draw(
        in: NSRect(x: 0, y: 0, width: pixels, height: pixels),
        from: .zero,
        operation: .sourceOver,
        fraction: 1
    )
    NSGraphicsContext.restoreGraphicsState()

    guard let png = rep.representation(using: .png, properties: [:]) else {
        throw CocoaError(.fileWriteUnknown)
    }
    try png.write(to: file)
}

for size in sizes {
    try render(size, to: iconset.appendingPathComponent("icon_\(size)x\(size).png"))
    try render(size * 2, to: iconset.appendingPathComponent("icon_\(size)x\(size)@2x.png"))
}
print("  rendered \(sizes.count * 2) sizes from \(source.lastPathComponent)")
