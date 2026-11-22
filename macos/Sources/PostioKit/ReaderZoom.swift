/// Stored as `[reader] zoom` in `config.toml`, the key GTK reads. Steps come from the boundary.
/// Body only: `WKWebView.pageZoom` scales text, images and the sender's sizes together and leaves
/// chrome alone.
public struct ReaderZoom: Equatable, Sendable {
    public let steps: [UInt16]
    public private(set) var percent: UInt16

    public init(percent: UInt16, steps: [UInt16]) {
        self.steps = steps.sorted()
        self.percent = percent
    }

    public var factor: Double { Double(percent) / 100 }

    public mutating func zoomIn() -> Bool {
        guard let next = steps.first(where: { $0 > percent }) else { return false }
        percent = next
        return true
    }

    public mutating func zoomOut() -> Bool {
        guard let next = steps.last(where: { $0 < percent }) else { return false }
        percent = next
        return true
    }

    public mutating func reset() -> Bool {
        guard percent != 100 else { return false }
        percent = 100
        return true
    }
}
