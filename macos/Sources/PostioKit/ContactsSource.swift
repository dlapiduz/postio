import Contacts
import Foundation
import PostioFFI

/// One person as the address book lends them: a name and their addresses.
public struct LentContact: Equatable, Sendable {
    public let name: String?
    public let addresses: [String]

    public init(name: String?, addresses: [String]) {
        self.name = name
        self.addresses = addresses
    }
}

/// The address book, as `ContactsSource` asks it: `SystemContactBook` on
/// the Mac, a fake in the tests.
@MainActor
public protocol ContactBook: AnyObject {
    /// What the person has said about Postio reading their contacts.
    var authorization: CNAuthorizationStatus { get }
    /// Ask them, once: the system's prompt, with `NSContactsUsageDescription`.
    func requestAccess() async -> Bool
    /// The people whose names hold `text`: names and email addresses only.
    func contacts(matching text: String) throws -> [LentContact]
}

/// The address book, lent to recipient completion (specs/009-focus-macos
/// T078; FR-051).
///
/// - **Asked once**, on the first keystroke in a recipient field: never at
///   launch, and never again once there is an answer. Denied (or
///   restricted) is an answer.
/// - **Lent, not kept.** Each answer reads the book again; nothing it lent
///   is stored, cached or logged (constitution VI). It crosses as `extra`
///   to `recipient_suggestions`, which uses it for that one answer.
/// - **Mail completes regardless.** Whatever the book says, the engine's
///   correspondents and groups are asked for, so a person who said no
///   still gets the addresses they have written to.
@MainActor
public final class ContactsSource {
    private let book: ContactBook
    /// Whether the prompt has been put up in this run.
    private var asked = false

    public init(book: ContactBook) {
        self.book = book
    }

    /// What the book lends for `text`, asking first if it never has.
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
        // A read that fails is a book that lends nothing this time.
        let people = (try? book.contacts(matching: text)) ?? []
        return people.flatMap { person in
            person.addresses.map { ExternalContactFfi(name: person.name, address: $0) }
        }
    }

    /// What completes `text`: the engine's answer, with what the book lent
    /// passed as `extra`. `engine` is `recipient_suggestions` for the
    /// draft's account; it blocks on the store, so it is run off the main
    /// actor.
    public func suggestions(
        for text: String,
        from engine: @escaping @Sendable (String, [ExternalContactFfi]) -> [RecipientSuggestionFfi]
    ) async -> [RecipientSuggestionFfi] {
        let extra = await lend(for: text)
        return await Task.detached { engine(text, extra) }.value
    }
}

/// The Mac's address book: `CNContactStore`, names and email addresses
/// only. Nothing it reads is kept past the call.
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
