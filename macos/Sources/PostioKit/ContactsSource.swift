import Contacts
import Foundation
import PostioFFI

public struct LentContact: Equatable, Sendable {
    public let name: String?
    public let addresses: [String]

    public init(name: String?, addresses: [String]) {
        self.name = name
        self.addresses = addresses
    }
}

@MainActor
public protocol ContactBook: AnyObject {
    var authorization: CNAuthorizationStatus { get }
    func requestAccess() async -> Bool
    func contacts(matching text: String) throws -> [LentContact]
}

/// Asked once, on the first keystroke in a recipient field, never at launch; denied is an answer.
/// Lent, not kept: each answer
/// reads the book again and nothing is stored, cached or logged (constitution VI). Mail completes
/// regardless of what the book says.
@MainActor
public final class ContactsSource {
    private let book: ContactBook
    private var asked = false

    public init(book: ContactBook) {
        self.book = book
    }

    public func lend(for text: String) async -> [ExternalContactFfi] {
        let text = text.trimmingCharacters(in: .whitespaces)
        guard !text.isEmpty else { return [] }
        switch book.authorization {
        case .notDetermined:
            guard !asked else { return [] }
            asked = true
            guard await book.requestAccess() else { return [] }
        case .denied, .restricted:
            return []
        default:
            break
        }
        let people = (try? book.contacts(matching: text)) ?? []
        return people.flatMap { person in
            person.addresses.map { ExternalContactFfi(name: person.name, address: $0) }
        }
    }

    /// `engine` blocks on the store, so it is run off the main actor.
    public func suggestions(
        for text: String,
        from engine: @escaping @Sendable (String, [ExternalContactFfi]) -> [RecipientSuggestionFfi]
    ) async -> [RecipientSuggestionFfi] {
        let extra = await lend(for: text)
        return await Task.detached { engine(text, extra) }.value
    }
}

/// Names and email addresses only; nothing it reads is kept past the call.
public final class SystemContactBook: ContactBook {
    private let store = CNContactStore()

    public init() {}

    public var authorization: CNAuthorizationStatus {
        CNContactStore.authorizationStatus(for: .contacts)
    }

    public func requestAccess() async -> Bool {
        (try? await store.requestAccess(for: .contacts)) ?? false
    }

    public func contacts(matching text: String) throws -> [LentContact] {
        let keys: [CNKeyDescriptor] = [
            CNContactFormatter.descriptorForRequiredKeys(for: .fullName),
            CNContactEmailAddressesKey as CNKeyDescriptor,
        ]
        let predicate = CNContact.predicateForContacts(matchingName: text)
        return try store.unifiedContacts(matching: predicate, keysToFetch: keys)
            .filter { !$0.emailAddresses.isEmpty }
            .map { person in
                LentContact(
                    name: CNContactFormatter.string(from: person, style: .fullName),
                    addresses: person.emailAddresses.map { $0.value as String })
            }
    }
}
