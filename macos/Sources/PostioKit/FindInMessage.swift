/// Find in the open message or conversation (spec 006 FR-018, #1705).
///
/// `find_in_message` (`⌘F`) opens the bar, `find_next` (`⌘G`) and
/// `find_previous` (`⇧⌘G`) step through the matches, and `Escape` closes it.
/// All of them are commands, so the state is here, where a command reaches
/// it, and the reader's web view acts on `request` -- WebKit's own find,
/// which matches across the sender's markup as it is drawn.
public struct FindInMessage: Equatable, Sendable {
    /// One search for the web view to carry out. A new `serial` is a new
    /// request, so two presses of `⌘G` are two steps.
    public struct Request: Equatable, Sendable {
        public let query: String
        public let backwards: Bool
        public let serial: Int
    }

    /// Whether the bar is on screen.
    public private(set) var isOpen = false
    /// What was typed. Kept when the bar closes, so `⌘G` finds it again.
    public private(set) var query = ""
    /// What the web view should search for now, or `nil` when nothing should
    /// be highlighted.
    public private(set) var request: Request?
    /// Bumped when the field should take the keyboard.
    public private(set) var focusToken = 0
    /// Whether the last search found nothing, for the bar to say so.
    public private(set) var missing = false

    private var serial = 0

    public init() {}

    /// `⌘F`: show the bar, and put the keyboard in it.
    public mutating func open() {
        isOpen = true
        focusToken += 1
    }

    /// What the field holds now. Searches as it is typed, forwards.
    public mutating func setQuery(_ text: String) {
        query = text
        missing = false
        ask(backwards: false)
    }

    /// `⌘G`. With nothing to find it opens the bar instead and answers `false`.
    public mutating func next() -> Bool { step(backwards: false) }

    /// `⇧⌘G`. As `next`, the other way.
    public mutating func previous() -> Bool { step(backwards: true) }

    /// `Escape`: the bar and its highlight go; the words stay.
    public mutating func close() {
        isOpen = false
        request = nil
        missing = false
    }

    /// What the web view reported for the last request.
    public mutating func found(_ any: Bool) {
        missing = !any
    }

    private mutating func step(backwards: Bool) -> Bool {
        guard !query.isEmpty else {
            open()
            return false
        }
        isOpen = true
        ask(backwards: backwards)
        return true
    }

    private mutating func ask(backwards: Bool) {
        guard !query.isEmpty else {
            request = nil
            return
        }
        serial += 1
        request = Request(query: query, backwards: backwards, serial: serial)
    }
}
