import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// File ▸ Import Contacts and Export Contacts (specs/011-focus-contacts S11):
/// a panel, the engine, then the sheet the engine said. The file is the
/// user's, so nothing is read without a panel and nothing is written without
/// one; the engine says every word.
@MainActor
struct ContactsIoModelTests {
    final class Engine: ContactsIoEngine {
        var imported: [Data] = []
        var invoked: [String] = []
        var importResult = ContactsImportResultFfi(sheet: ContactsIoModelTests.sheet(review: true), error: nil)
        var exportResult = ContactsExportResultFfi(
            text: "BEGIN:VCARD\r\nEND:VCARD\r\n",
            sheet: ContactsIoModelTests.sheet(review: false, title: "Exported 1 contact"), error: nil)
        func contactsPanelWords() -> ContactsPanelWordsFfi {
            ContactsPanelWordsFfi(
                importTitle: "Import Contacts", importMessage: "Choose a .vcf file.",
                exportTitle: "Export Contacts", exportMessage: "Postio writes one .vcf.",
                exportName: "Contacts.vcf")
        }
        func contactsImport(_ bytes: Data) -> ContactsImportResultFfi {
            imported.append(bytes)
            return importResult
        }
        func contactsExport() -> ContactsExportResultFfi { exportResult }
        func demoContactsFile() -> Data { Data("DEMO".utf8) }
        func invoke(_ id: String) { invoked.append(id) }
    }

    @MainActor final class Panels: ContactsFilePanels {
        var cards: URL?
        var destination: URL?
        var asked: [String] = []
        func chooseCards(_ words: ContactsPanelWordsFfi) -> URL? {
            asked.append(words.importTitle)
            return cards
        }
        func chooseDestination(_ words: ContactsPanelWordsFfi) -> URL? {
            asked.append(words.exportTitle)
            return destination
        }
    }

    nonisolated static func sheet(review: Bool, title: String = "Imported 3 contacts") -> ContactsIoSheetFfi {
        ContactsIoSheetFfi(
            title: title, lines: ["2 new people added", "An import cannot be undone."],
            review: review ? EditButtonFfi(label: "Review", key: "g w") : nil,
            done: EditButtonFfi(label: "Done", key: "Escape"))
    }

    func scratch() throws -> URL {
        let url = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        return url
    }

    @Test func anImportReadsTheFileThePersonChoseAndShowsWhatTheEngineSaid() throws {
        let folder = try scratch()
        let file = folder.appendingPathComponent("cards.vcf")
        try Data("BEGIN:VCARD".utf8).write(to: file)
        let engine = Engine()
        let panels = Panels()
        panels.cards = file
        let model = ContactsIoModel(engine: engine, panels: panels, temporary: folder)
        model.runImport()
        #expect(panels.asked == ["Import Contacts"])
        #expect(engine.imported == [Data("BEGIN:VCARD".utf8)])
        #expect(model.sheet?.title == "Imported 3 contacts")
        #expect(model.isOpen)
    }

    @Test func cancellingThePanelReadsNothingAndShowsNothing() {
        let engine = Engine()
        let model = ContactsIoModel(engine: engine, panels: Panels())
        model.runImport()
        #expect(engine.imported.isEmpty && model.sheet == nil)
    }

    @Test func aFileThatCannotBeReadSaysSoAndImportsNothing() {
        let engine = Engine()
        let panels = Panels()
        panels.cards = URL(fileURLWithPath: "/nonexistent/cards.vcf")
        let model = ContactsIoModel(engine: engine, panels: panels)
        model.runImport()
        #expect(engine.imported.isEmpty)
        #expect(model.error == "Postio could not read that file.")
    }

    @Test func reviewClosesTheSheetAndAsksForTheReviewAndDoneJustCloses() {
        let engine = Engine()
        let panels = Panels()
        let model = ContactsIoModel(engine: engine, panels: panels, demo: true)
        model.runImport()
        #expect(engine.imported == [Data("DEMO".utf8)], "a demo reads its own file, not a panel")
        #expect(panels.asked.isEmpty)
        model.review()
        #expect(model.sheet == nil)
        #expect(engine.invoked == ["contacts_suggestions"])

        model.runImport()
        model.done()
        #expect(model.sheet == nil)
        #expect(engine.invoked == ["contacts_suggestions"], "Done asks for nothing")
    }

    @Test func noReviewButtonMeansNoReview() {
        let engine = Engine()
        engine.importResult = ContactsImportResultFfi(sheet: Self.sheet(review: false), error: nil)
        let model = ContactsIoModel(engine: engine, panels: Panels(), demo: true)
        model.runImport()
        model.review()
        #expect(engine.invoked.isEmpty && model.isOpen, "nothing to review, so the sheet stays")
    }

    @Test func anExportWritesTheTextWhereThePersonChoseAndThenSaysHowMany() throws {
        let folder = try scratch()
        let target = folder.appendingPathComponent("mine.vcf")
        let engine = Engine()
        let panels = Panels()
        panels.destination = target
        let model = ContactsIoModel(engine: engine, panels: panels, temporary: folder)
        model.runExport()
        #expect(panels.asked == ["Export Contacts"])
        #expect(try String(contentsOf: target, encoding: .utf8) == "BEGIN:VCARD\r\nEND:VCARD\r\n")
        #expect(model.sheet?.title == "Exported 1 contact")
    }

    @Test func cancellingTheSavePanelWritesNothingAndShowsNothing() throws {
        let folder = try scratch()
        let model = ContactsIoModel(engine: Engine(), panels: Panels(), temporary: folder)
        model.runExport()
        #expect(model.sheet == nil)
        #expect(try FileManager.default.contentsOfDirectory(atPath: folder.path).isEmpty)
    }

    @Test func aDemoExportWritesUnderItsOwnFolderWithoutAPanel() throws {
        let folder = try scratch()
        let panels = Panels()
        let model = ContactsIoModel(engine: Engine(), panels: panels, demo: true, temporary: folder)
        model.runExport()
        #expect(panels.asked.isEmpty)
        #expect(FileManager.default.fileExists(atPath: folder.appendingPathComponent("Contacts.vcf").path))
        #expect(model.sheet != nil)
    }
}
