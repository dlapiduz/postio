import PostioFFI

/// Where `space` has paged the reader to, and when the page should scroll.
///
/// Two numbers with different jobs. `page` is which scroll anchor the reader
/// is on (`ReaderPaging`). `token` is the web view's cue to *go* there: it
/// moves only when a page turn is asked for, because a new token is a scroll
/// of whatever document is on screen at that moment.
public struct ReaderPages: Equatable, Sendable {
    public private(set) var page: UInt32 = 0
    public private(set) var token = 0

    public init() {}

    /// `space` or `⇧space`: the next or previous anchor, scrolled to now.
    public mutating func turn(forward: Bool) {
        page = readerPageAfter(current: page, forward: forward)
        token += 1
    }

    /// The cursor moved to another message: the next turn starts from the
    /// top. **No scroll** -- the page on screen is still the last message's
    /// until the new one loads, and a fresh load is at the top already.
    public mutating func newMessage() {
        page = 0
    }
}
