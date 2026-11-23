/// A type, not `@State`, because `unsubscribe` is a command (`X`, the palette) and a command cannot
/// reach view state (#1706). Holds no consent: whether there is a list to leave is `messageFacts`'
/// and what an activation records is re-derived from the message id (`activateUnsubscribe`).
public struct Unsubscribing: Equatable, Sendable {
    public enum State: Equatable, Sendable {
        case offered
        /// It writes, and a write can queue behind a sync's commit, so the banner says what it is
        /// doing.
        case leaving
        /// The banner stays to offer the one action `[focus] after_unsubscribe` names (#1952); not
        /// done until pressed.
        case clearing(Sweep)
        case left
        case failed(String)
    }

    public struct Sweep: Equatable, Sendable {
        public let summary: String
        public let button: String
        public let delete: Bool

        public init(summary: String, button: String, delete: Bool) {
            self.summary = summary
            self.button = button
            self.delete = delete
        }
    }

    private var states: [Int64: State] = [:]

    public init() {}

    public func state(of message: Int64) -> State { states[message] ?? .offered }

    /// `false` when it would be a second activation: one in flight, or a list already left.
    public mutating func begin(_ message: Int64) -> Bool {
        switch state(of: message) {
        case .leaving, .left, .clearing:
            return false
        case .offered, .failed:
            states[message] = .leaving
            return true
        }
    }

    public mutating func finish(_ message: Int64, complaint: String?) {
        states[message] = complaint.map(State.failed) ?? .left
    }

    public mutating func offerSweep(_ message: Int64, _ sweep: Sweep) {
        guard state(of: message) == .left else { return }
        states[message] = .clearing(sweep)
    }

    public mutating func answerSweep(_ message: Int64) {
        if case .clearing = state(of: message) { states[message] = .left }
    }

    public mutating func withdraw(_ message: Int64) {
        states[message] = nil
    }

    public mutating func clear() { states = [:] }
}
