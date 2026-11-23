import Foundation
import Observation
import PostioFFI

/// The remedy is the boundary's: a store from another build or under another key fails every retry,
/// so the page offers a fresh store (`start_over`); anything else (another Postio holds it, a
/// locked Keychain (ADR 0014), no runtime) may open next time, so it offers to try again. Words are
/// GTK's (`store_refusal_words`).
public struct StoreRefusal: Equatable, Sendable {
    public enum Remedy: Equatable, Sendable {
        case tryAgain
        case startOver
    }

    public let heading: String
    public let sentence: String
    public let button: String
    public let remedy: Remedy

    public init(_ error: Error, words: StoreRefusalWordsFfi) {
        switch error as? SessionError {
        case .StoreFromAnotherBuild:
            // GTK's page says what a fresh store keeps and what stays behind, not the store layer's
            // schema sentence.
            self.init(
                heading: words.fromAnotherBuild, sentence: words.startOverSentence,
                button: words.startOver, remedy: .startOver)
        case .StoreUnderAnotherKey:
            // The key does not change between attempts (#1598); nothing in the store decrypts, so
            // the page promises no accounts.
            self.init(
                heading: words.underAnotherKey,
                sentence: words.startOverUnderAnotherKeySentence,
                button: words.startOver, remedy: .startOver)
        default:
            self.init(
                heading: words.cantOpen, sentence: Self.sentence(of: error),
                button: words.tryAgain, remedy: .tryAgain)
        }
    }

    init(heading: String, sentence: String, button: String, remedy: Remedy) {
        self.heading = heading
        self.sentence = sentence
        self.button = button
        self.remedy = remedy
    }

    static func sentence(of error: Error) -> String {
        switch error as? SessionError {
        case let .StoreUnavailable(message), let .KeyringLocked(message),
             let .StoreFromAnotherBuild(message), let .StoreUnderAnotherKey(message),
             let .RuntimeUnavailable(message):
            return message
        case nil:
            return error.localizedDescription
        }
    }
}

/// "Start a fresh store" sets it aside off the main actor (`start_over` blocks on the Keychain and
/// disk), then opens the fresh one. A failed start over says why and offers to try again, as GTK's
/// page does.
@MainActor
@Observable
public final class StoreRefusalModel {
    public private(set) var refusal: StoreRefusal
    public private(set) var working = false

    private let words: StoreRefusalWordsFfi
    private let startOver: @Sendable () throws -> StartedOverFfi
    private let reopen: @MainActor (StartedOverFfi?) -> Void

    public init(
        refusal: StoreRefusal, words: StoreRefusalWordsFfi,
        startOver: @escaping @Sendable () throws -> StartedOverFfi,
        reopen: @escaping @MainActor (StartedOverFfi?) -> Void
    ) {
        self.refusal = refusal
        self.words = words
        self.startOver = startOver
        self.reopen = reopen
    }

    public var button: String { working ? words.startingOver : refusal.button }

    public func act() async {
        guard !working else { return }
        switch refusal.remedy {
        case .tryAgain:
            reopen(nil)
        case .startOver:
            working = true
            let startOver = startOver
            let answer = await Task.detached { Result { try startOver() } }.value
            working = false
            switch answer {
            case let .success(started):
                reopen(started)
            case let .failure(error):
                refusal = StoreRefusal(
                    heading: words.cantOpen, sentence: StoreRefusal.sentence(of: error),
                    button: words.tryAgain, remedy: .tryAgain)
            }
        }
    }
}
