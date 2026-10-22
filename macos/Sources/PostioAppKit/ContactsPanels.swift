import AppKit
import PostioFFI
import PostioKit
import UniformTypeIdentifiers

/// The open and save panels for a vCard (specs/011-focus-contacts S11). The
/// words are the engine's (`ContactsPanelWordsFfi`); the file is the user's.
@MainActor
public struct ContactsPanels: ContactsFilePanels {
    public init() {}

    public func chooseCards(_ words: ContactsPanelWordsFfi) -> URL? {
        // POSTIO-CONSENT: a file is read only because somebody chose it in
        // this open panel; it is parsed on this Mac and nothing is uploaded.
        let panel = NSOpenPanel()
        panel.title = words.importTitle
        panel.message = words.importMessage
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [UTType(filenameExtension: "vcf") ?? .data]
        return panel.runModal() == .OK ? panel.url : nil
    }

    public func chooseDestination(_ words: ContactsPanelWordsFfi) -> URL? {
        // POSTIO-CONSENT: a file is written only to the name and place
        // somebody chose in this save panel.
        let panel = NSSavePanel()
        panel.title = words.exportTitle
        panel.message = words.exportMessage
        panel.nameFieldStringValue = words.exportName
        panel.allowedContentTypes = [UTType(filenameExtension: "vcf") ?? .data]
        return panel.runModal() == .OK ? panel.url : nil
    }
}
