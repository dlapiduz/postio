/// Commands, so the state is here where a command reaches it; the reader's web view acts on
/// `request`.
public struct FindInMessage: Equatable, Sendable {
    /// A new `serial` is a new request, so two presses of `⌘G` are two steps.
    public struct Request: Equatable, Sendable {
        public let query: String
        public let backwards: Bool
        public let serial: Int
        /// Stops at the last match instead of going round: finding nothing then means the message's
        /// matches ran out, and the window goes on to the next matched message.
        public let bounded: Bool
    }

    public private(set) var isOpen = false
    public private(set) var query = ""
    public private(set) var request: Request?
    public private(set) var focusToken = 0
    public private(set) var missing = false

    private var serial = 0

    public init() {}

    public mutating func open() {
        isOpen = true
        focusToken += 1
    }

    public mutating func setQuery(_ text: String) {
        query = text
        missing = false
        ask(backwards: false)
    }

    public mutating func next() -> Bool { step(backwards: false) }

    public mutating func previous() -> Bool { step(backwards: true) }

    public mutating func nextWithin(_ words: String) { stepWithin(words, backwards: false) }

    public mutating func previousWithin(_ words: String) { stepWithin(words, backwards: true) }

    /// Finding nothing is not an edge here: the match may be in a file.
    public mutating func arrive(_ words: String, backwards: Bool) {
        if query.isEmpty { query = words }
        missing = false
        ask(backwards: backwards, bounded: false)
    }

    private mutating func stepWithin(_ words: String, backwards: Bool) {
        if query.isEmpty { query = words }
        guard !query.isEmpty else { return }
        ask(backwards: backwards, bounded: true)
    }

    public mutating func close() {
        isOpen = false
        request = nil
        missing = false
    }

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

    private mutating func ask(backwards: Bool, bounded: Bool = false) {
        guard !query.isEmpty else {
            request = nil
            return
        }
        serial += 1
        request = Request(query: query, backwards: backwards, serial: serial, bounded: bounded)
    }
}
