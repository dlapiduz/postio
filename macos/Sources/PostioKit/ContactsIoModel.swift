import Foundation
import Observation
import PostioFFI

/// What importing and exporting contacts asks of the engine
/// (specs/011-focus-contacts S11). `PostioSession` is the real answer; the
/// protocol is what lets a test drive `ContactsIoModel` without a store.
public protocol ContactsIoEngine: AnyObject {
    /// What the open and save panels say.
    func contactsPanelWords() -> ContactsPanelWordsFfi
    /// Reads and applies the cards in a file the user chose.
    func contactsImport(_ bytes: Data) -> ContactsImportResultFfi
    /// One `.vcf` of the people the Contacts list holds.
    func contactsExport() -> ContactsExportResultFfi
    /// The file a demo imports from in place of the open panel.
    func demoContactsFile() -> Data
    /// A registry command, as a key press would send it.
    func invoke(_ id: String)
}

extension PostioSession: ContactsIoEngine {}

/// The panels, behind a protocol so a test presents none. The file is the
/// user's: it is read or written only through a panel they used.
@MainActor
public protocol ContactsFilePanels {
    /// An open panel for a `.vcf`; `nil` when they cancel.
    func chooseCards(_ words: ContactsPanelWordsFfi) -> URL?
    /// A save panel for the export; `nil` when they cancel.
    func chooseDestination(_ words: ContactsPanelWordsFfi) -> URL?
}

/// File ▸ Import Contacts and Export Contacts: the panel, the engine, and the
/// sheet after. Parse, apply, write-out and every word are Rust's; this holds
/// the sheet the engine last said and sends back a click on its buttons. No
/// AppKit (#1264): `ContactsPanels` presents the panels.
@MainActor
@Observable
public final class ContactsIoModel {
    /// The sheet over the window; `nil` while there is none.
    public private(set) var sheet: ContactsIoSheetFfi?
    /// Why the last attempt did nothing, in words for the screen.
    public private(set) var error: String?

    @ObservationIgnored private let engine: ContactsIoEngine
    @ObservationIgnored private let panels: ContactsFilePanels
    @ObservationIgnored private let demo: Bool
    @ObservationIgnored private let temporary: URL

    /// `demo`: read the demo's own file and write under `temporary`, in place
    /// of the panels, which a screenshot cannot answer.
    public init(
        engine: ContactsIoEngine, panels: ContactsFilePanels, demo: Bool = false,
        temporary: URL = FileManager.default.temporaryDirectory
    ) {
        self.engine = engine
        self.panels = panels
        self.demo = demo
        self.temporary = temporary
    }

    /// File ▸ Import Contacts: an open panel, then the engine reads the file.
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

    /// File ▸ Export Contacts: the engine writes the text, a save panel says
    /// where it goes.
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

    /// Review: the sheet goes and the merge sheet comes, as `g w` would.
    public func review() {
        guard sheet?.review != nil else { return }
        sheet = nil
        engine.invoke("contacts_suggestions")
    }

    /// Done, or Escape: the sheet goes.
    public func done() {
        sheet = nil
    }

    /// Whether the sheet is up.
    public var isOpen: Bool { sheet != nil }
}
