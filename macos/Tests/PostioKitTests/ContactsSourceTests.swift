import Contacts
import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The address book, lent to recipient completion (specs/009-focus-macos
/// T077; spec 009 "Contacts").
///
/// Contacts is asked once, on the first keystroke in a recipient field --
/// never at launch, never again after an answer -- and what it lends is
/// names and addresses for one answer, kept nowhere. Mail's own
/// correspondents complete whatever Contacts said.
@MainActor
struct ContactsSourceTests {
    /// An address book whose authorization a test sets, counting the times
    /// it is asked for access and read.
    final class Book: ContactBook {
        var authorization: CNAuthorizationStatus
        /// What a request for access answers, and becomes.
        var grants: Bool
        var people: [LentContact]
        private(set) var asked = 0
        private(set) var reads: [String] = []

        init(_ authorization: CNAuthorizationStatus, grants: Bool = true, people: [LentContact] = []) {
            self.authorization = authorization
            self.grants = grants
            self.people = people
        }

        func requestAccess() async -> Bool {
            asked += 1
            authorization = grants ? .authorized : .denied
            return grants
        }

        func contacts(matching text: String) throws -> [LentContact] {
            reads.append(text)
            return people.filter { $0.name?.localizedCaseInsensitiveContains(text) ?? false }
        }
    }

    static let ada = LentContact(name: "Ada Example", addresses: ["ada@example.com", "ada@work.example"])

    /// What the engine answers, recording the `extra` it was lent.
    final class Mail: @unchecked Sendable {
        private(set) var lent: [[ExternalContactFfi]] = []
        func suggestions(_ text: String, _ extra: [ExternalContactFfi]) -> [RecipientSuggestionFfi] {
            lent.append(extra)
            return [RecipientSuggestionFfi(label: "Ada Correspondent <ada@mail.example>", accepted: "ada@mail.example, ", group: false)]
        }
    }

    @Test func notDeterminedAsksOnTheFirstKeystrokeAndOnlyOnce() async {
        let book = Book(.notDetermined, people: [Self.ada])
        let source = ContactsSource(book: book)
        #expect(book.asked == 0, "nothing is asked before a recipient is typed")

        let first = await source.lend(for: "Ad")
        #expect(book.asked == 1, "the first keystroke asks")
        #expect(first.map(\.address) == ["ada@example.com", "ada@work.example"])

        _ = await source.lend(for: "Ada")
        _ = await source.lend(for: "Ada E")
        #expect(book.asked == 1, "and never again")
    }

    @Test func aRefusalAtThePromptIsNotAskedAgain() async {
        let book = Book(.notDetermined, grants: false, people: [Self.ada])
        let source = ContactsSource(book: book)
        #expect(await source.lend(for: "Ad").isEmpty)
        #expect(await source.lend(for: "Ada").isEmpty)
        #expect(book.asked == 1)
        #expect(book.reads.isEmpty, "a refused book is never read")
    }

    @Test func deniedNeverAsksAndMailStillCompletes() async {
        let book = Book(.denied, people: [Self.ada])
        let source = ContactsSource(book: book)
        let mail = Mail()
        let suggestions = await source.suggestions(for: "Ad") { text, extra in
            mail.suggestions(text, extra)
        }
        #expect(book.asked == 0, "denied is an answer: never asked again")
        #expect(book.reads.isEmpty)
        #expect(mail.lent == [[]], "the engine is asked with nothing lent")
        #expect(suggestions.map(\.accepted) == ["ada@mail.example, "], "mail's correspondents still complete")
    }

    @Test func restrictedIsTreatedAsDenied() async {
        let book = Book(.restricted, people: [Self.ada])
        let source = ContactsSource(book: book)
        #expect(await source.lend(for: "Ad").isEmpty)
        #expect(book.asked == 0)
    }

    @Test func authorizedLendsNamesAndAddressesForTheAnswerOnly() async {
        let book = Book(.authorized, people: [Self.ada])
        let source = ContactsSource(book: book)
        let mail = Mail()
        _ = await source.suggestions(for: "Ad") { text, extra in mail.suggestions(text, extra) }
        #expect(mail.lent == [[
            ExternalContactFfi(name: "Ada Example", address: "ada@example.com"),
            ExternalContactFfi(name: "Ada Example", address: "ada@work.example"),
        ]])
        #expect(book.reads == ["Ad"])
        // Kept nowhere: a second answer reads the book again.
        _ = await source.suggestions(for: "Ad") { text, extra in mail.suggestions(text, extra) }
        #expect(book.reads == ["Ad", "Ad"])
    }

    @Test func aBlankFieldReadsNothing() async {
        let book = Book(.notDetermined, people: [Self.ada])
        let source = ContactsSource(book: book)
        #expect(await source.lend(for: "  ").isEmpty)
        #expect(book.asked == 0, "an empty field is not a keystroke in a recipient")
    }

    @Test func theUsageDescriptionIsInTheBundle() throws {
        let plist = URL(fileURLWithPath: #filePath)
            .deletingLastPathComponent()  // PostioKitTests
            .deletingLastPathComponent()  // Tests
            .deletingLastPathComponent()  // macos
            .appendingPathComponent("Resources/Info.plist")
        let data = try Data(contentsOf: plist)
        let info = try #require(
            try PropertyListSerialization.propertyList(from: data, format: nil) as? [String: Any])
        let said = try #require(info["NSContactsUsageDescription"] as? String)
        #expect(!said.isEmpty)
    }
}
