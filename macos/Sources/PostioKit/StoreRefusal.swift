import Foundation
import Observation
import PostioFFI

/// What the window says when the store will not open on launch
/// (specs/009-focus-macos T100), and which way forward it offers.
///
/// The remedy is the boundary's, from the case it answers: a store written
/// by another build is `StoreFromAnotherBuild`, which trying again meets
/// every time, so the page offers a fresh store (`start_over`); anything
/// else -- another Postio has the store open, a locked Keychain (ADR 0014),
/// no runtime -- may open next time, so it offers to try again. The words
/// are GTK's refusal page's (`store_refusal_words`), and the sentence for a
/// store that may open is the store layer's own, never a debug description.
public struct StoreRefusal: Equatable, Sendable {
    /// The way forward the page offers.
    public enum Remedy: Equatable, Sendable {
        /// Open the store again.
        case tryAgain
        /// Set the store aside and start a fresh one, then open that.
        case startOver
    }

    public let heading: String
    public let sentence: String
    public let button: String
    public let remedy: Remedy

    public init(_ error: Error, words: StoreRefusalWordsFfi) {
        if case .StoreFromAnotherBuild = error as? SessionError {
            // GTK's page says what a fresh store keeps and what stays behind
            // in place of the store layer's sentence, which names a schema.
            self.init(
                heading: words.fromAnotherBuild, sentence: words.startOverSentence,
                button: words.startOver, remedy: .startOver)
        } else {
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

    /// The boundary's sentence for `error`: every `SessionError` carries one,
    /// already written for a person.
    static func sentence(of error: Error) -> String {
        switch error as? SessionError {
        case let .StoreUnavailable(message), let .KeyringLocked(message),
             let .StoreFromAnotherBuild(message), let .RuntimeUnavailable(message):
            return message
        case nil:
            return error.localizedDescription
        }
    }
}

/// The refusal page's one button, run: "Try again" opens the store again;
/// "Start a fresh store" sets it aside off the main actor (`start_over`
/// blocks on the Keychain and the disk), then opens the fresh one. A start
/// over that fails says why and offers to try again, which opens the store
/// again and, if it is still the old one, offers the start over again --
/// GTK's page, step for step.
@MainActor
@Observable
public final class StoreRefusalModel {
    /// What the page says now.
    public private(set) var refusal: StoreRefusal
    /// Whether a start over is running: the button says so and holds.
    public private(set) var working = false

    private let words: StoreRefusalWordsFfi
    private let startOver: @Sendable () throws -> StartedOverFfi
    private let reopen: @MainActor (StartedOverFfi?) -> Void

    /// `reopen` opens the store again, told what a start over set aside
    /// when one did.
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

    /// The button's words.
    public var button: String { working ? words.startingOver : refusal.button }

    /// Press the button. A press while a start over runs does nothing.
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
