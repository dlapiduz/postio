import Contacts
import Foundation
import PostioFFI

/// The Mac's address book behind sync: `CNContactStore`, read only
/// (specs/011-focus-contacts S12). The second file that imports `Contacts`,
/// after `ContactsSource.swift`.
///
/// **Nothing runs this in a test, a demo or a storyboard.** It is constructed
/// only by the app when a person presses Turn on and preview, and it refuses to
/// touch the store at all in a demo or under a test runner
/// (`refusesHere`), so a mistake elsewhere cannot reach anyone's real address
/// book. It has no write: `CNSaveRequest` is not used, anywhere.
@MainActor
public final class AppleAddressBook: SyncAddressBook {
    private let store = CNContactStore()

    public init() {}

    /// Whether this process must not touch the real store: a demo (which
    /// invents everything), or anything that is not the application itself.
    /// A test runner, a command-line tool and a preview all run outside
    /// `Postio.app`, and the real address book is the application's alone, so
    /// this asks the one question that cannot be fooled by how a runner names
    /// itself.
    public static var refusesHere: Bool {
        DemoMode.seed != nil || !Bundle.main.bundlePath.hasSuffix(".app")
    }

    public var access: SyncAccess {
        if Self.refusesHere { return .denied }
        switch CNContactStore.authorizationStatus(for: .contacts) {
        case .notDetermined: return .notDetermined
        case .authorized: return .authorized
        default: return .denied
        }
    }

    public func requestAccess() async -> SyncAccess {
        guard !Self.refusesHere else { return .denied }
        let granted = (try? await store.requestAccess(for: .contacts)) ?? false
        return granted ? .authorized : .denied
    }

    public func snapshot() throws -> [ExternalRecordFfi] {
        guard !Self.refusesHere, access == .authorized else { return [] }
        let keys: [CNKeyDescriptor] = [
            CNContactIdentifierKey as CNKeyDescriptor,
            CNContactGivenNameKey as CNKeyDescriptor,
            CNContactFamilyNameKey as CNKeyDescriptor,
            CNContactEmailAddressesKey as CNKeyDescriptor,
            CNContactPhoneNumbersKey as CNKeyDescriptor,
            CNContactOrganizationNameKey as CNKeyDescriptor,
            CNContactJobTitleKey as CNKeyDescriptor,
        ]
        var records: [ExternalRecordFfi] = []
        try store.enumerateContacts(with: CNContactFetchRequest(keysToFetch: keys)) { person, _ in
            records.append(
                ExternalRecordFfi(
                    id: person.identifier, given: person.givenName, family: person.familyName,
                    emails: person.emailAddresses.map { $0.value as String },
                    phones: person.phoneNumbers.map { $0.value.stringValue },
                    organization: person.organizationName, title: person.jobTitle))
        }
        return records
    }
}
