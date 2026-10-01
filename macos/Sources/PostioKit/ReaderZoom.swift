/// How large the reader draws message bodies.
///
/// One reader-wide preference (spec 006 FR-021d), kept in `config.toml` as
/// `[reader] zoom` -- the same key GTK reads, so a zoom set on one frontend is
/// the zoom on the other. It moves in the design's fixed steps (FR-021b),
/// which come from the boundary rather than being written again here.
///
/// The body only: `WKWebView.pageZoom` scales text, images and the sender's
/// own sizes together and re-flows the layout, and leaves the app's chrome
/// alone (FR-021a).
public struct ReaderZoom: Equatable, Sendable {
    /// The steps, ascending.
    public let steps: [UInt16]
    /// The zoom, in percent.
    public private(set) var percent: UInt16

    public init(percent: UInt16, steps: [UInt16]) {
        self.steps = steps.sorted()
        self.percent = percent
    }

    /// What `WKWebView.pageZoom` is given.
    public var factor: Double { Double(percent) / 100 }

    /// The next step up. `false` at the largest, where nothing moved.
    public mutating func zoomIn() -> Bool {
        guard let next = steps.first(where: { $0 > percent }) else { return false }
        percent = next
        return true
    }

    /// The next step down. `false` at the smallest.
    public mutating func zoomOut() -> Bool {
        guard let next = steps.last(where: { $0 < percent }) else { return false }
        percent = next
        return true
    }

    /// Back to 100%. `false` when already there.
    public mutating func reset() -> Bool {
        guard percent != 100 else { return false }
        percent = 100
        return true
    }
}
