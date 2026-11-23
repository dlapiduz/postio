import PostioFFI

/// `page` is the scroll anchor the reader is on (`ReaderPaging`). `token` is the web view's cue to
/// go there: it moves only when a page turn is asked for, since a new token scrolls whatever is on
/// screen.
public struct ReaderPages: Equatable, Sendable {
    public private(set) var page: UInt32 = 0
    public private(set) var token = 0

    public init() {}

    public mutating func turn(forward: Bool) {
        page = readerPageAfter(current: page, forward: forward)
        token += 1
    }

    /// The next turn starts from the top. No scroll: the page on screen is the last message's until
    /// the new one loads, which starts at the top.
    public mutating func newMessage() {
        page = 0
    }
}
