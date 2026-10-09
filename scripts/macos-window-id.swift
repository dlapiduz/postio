// Print the window number of process <pid>'s largest on-screen window, for
// `screencapture -l` (scripts/macos-shot.sh). CoreGraphics' window list
// needs no Accessibility permission; capturing it needs Screen Recording.
import CoreGraphics
import Foundation

guard CommandLine.arguments.count == 2, let pid = Int32(CommandLine.arguments[1]) else {
    FileHandle.standardError.write("usage: macos-window-id.swift <pid>\n".data(using: .utf8)!)
    exit(2)
}
let windows = CGWindowListCopyWindowInfo([.optionOnScreenOnly], kCGNullWindowID) as? [[String: Any]] ?? []
let mine = windows.filter { ($0[kCGWindowOwnerPID as String] as? Int32) == pid && ($0[kCGWindowLayer as String] as? Int) == 0 }
let largest = mine.max { a, b in
    func area(_ w: [String: Any]) -> Double {
        let bounds = w[kCGWindowBounds as String] as? [String: Double] ?? [:]
        return (bounds["Width"] ?? 0) * (bounds["Height"] ?? 0)
    }
    return area(a) < area(b)
}
guard let number = largest?[kCGWindowNumber as String] as? Int else { exit(1) }
print(number)
