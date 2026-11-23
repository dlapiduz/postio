import Foundation
import Observation
import PostioFFI

public protocol ContactsIoEngine: AnyObject {
    func contactsPanelWords() -> ContactsPanelWordsFfi
    func contactsImport(_ bytes: Data) -> ContactsImportResultFfi
    func contactsExport() -> ContactsExportResultFfi
    func demoContactsFile() -> Data
    func invoke(_ id: String)
}

extension PostioSession: ContactsIoEngine {}

/// The file is the user's: read or written only through a panel they used.
@MainActor
public protocol ContactsFilePanels {
    func chooseCards(_ words: ContactsPanelWordsFfi) -> URL?
    func chooseDestination(_ words: ContactsPanelWordsFfi) -> URL?
}

@MainActor
@Observable
public final class ContactsIoModel {
    public private(set) var sheet: ContactsIoSheetFfi?
    public private(set) var error: String?

    @ObservationIgnored private let engine: ContactsIoEngine
    @ObservationIgnored private let panels: ContactsFilePanels
    @ObservationIgnored private let demo: Bool
    @ObservationIgnored private let temporary: URL

    public init(
        engine: ContactsIoEngine, panels: ContactsFilePanels, demo: Bool = false,
        temporary: URL = FileManager.default.temporaryDirectory
    ) {
        self.engine = engine
        self.panels = panels
        self.demo = demo
        self.temporary = temporary
    }

    public func runImport() {
        let words = engine.contactsPanelWords()
        let bytes: Data
        if demo {
            bytes = engine.demoContactsFile()
        } else {
            // POSTIO-CONSENT: a file is read only because somebody chose it
            // in an open panel (`ContactsPanels`); it is parsed on this Mac
            // and nothing is uploaded.
            guard let url = panels.chooseCards(words) else { return }
            guard let read = try? Data(contentsOf: url) else {
                error = "Postio could not read that file."
                return
            }
            bytes = read
        }
        let result = engine.contactsImport(bytes)
        error = result.error
        sheet = result.sheet
    }

    public func runExport() {
        let words = engine.contactsPanelWords()
        let result = engine.contactsExport()
        error = result.error
        guard let text = result.text, let shown = result.sheet else { return }
        let destination: URL
        if demo {
            destination = temporary.appendingPathComponent(words.exportName)
        } else {
            // POSTIO-CONSENT: a file is written only to the name and place
            // somebody chose in a save panel (`ContactsPanels`).
            guard let chosen = panels.chooseDestination(words) else { return }
            destination = chosen
        }
        do {
            try Data(text.utf8).write(to: destination, options: .atomic)
            sheet = shown
        } catch {
            self.error = "Postio could not write that file."
        }
    }

    public func review() {
        guard sheet?.review != nil else { return }
        sheet = nil
        engine.invoke("contacts_suggestions")
    }

    public func done() {
        sheet = nil
    }

    public var isOpen: Bool { sheet != nil }
}
