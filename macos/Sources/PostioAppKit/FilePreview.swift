import AppKit
import PostioFFI
import PostioKit
import QuickLookUI

/// What the Files tab hands the system (specs/010-focus-search T130;
/// design §3.8, FR-031, FR-053): the system's own Quick Look on a copy of
/// the file, or a save panel to put it somewhere.
///
/// The copy is the controller's: written into the app's own temporary
/// folder when Space or ⌘↓ asked for it (`FocusFileCopy`), and removed
/// when this says the panel is gone (`focusSearchFileDone`). The panel is
/// handed a file URL in that folder and never anything else; nothing is
/// fetched and nothing leaves the machine.
@MainActor
public final class FilePreview: NSObject {
    /// Told when the person closed Quick Look, or the save panel is done.
    private let done: () -> Void
    /// Shows the save panel; a test replaces it.
    private let save: (NSSavePanel, NSWindow?, @escaping (NSApplication.ModalResponse) -> Void) -> Void

    /// The copy Quick Look is showing, while it is up.
    public private(set) var url: URL?

    public init(
        done: @escaping () -> Void,
        save: @escaping (NSSavePanel, NSWindow?, @escaping (NSApplication.ModalResponse) -> Void) -> Void = {
            panel, window, answer in
            if let window {
                panel.beginSheetModal(for: window, completionHandler: answer)
            } else {
                answer(panel.runModal())
            }
        }
    ) {
        self.done = done
        self.save = save
    }

    /// Show what `FocusFileCopy` said, over `window`.
    public func apply(_ copy: FileCopyFfi?, over window: NSWindow?) {
        guard let copy else {
            // The controller closed it: the copy is its to remove, so this
            // says nothing back.
            let panel = QLPreviewPanel.sharedPreviewPanelExists() ? QLPreviewPanel.shared() : nil
            url = nil
            if panel?.isVisible == true { panel?.orderOut(nil) }
            return
        }
        let source = URL(fileURLWithPath: copy.path)
        if copy.save {
            offerToSave(source, named: copy.name, over: window)
        } else {
            url = source
            let panel = QLPreviewPanel.shared()
            if panel?.isVisible == true {
                panel?.reloadData()
            } else {
                panel?.makeKeyAndOrderFront(nil)
            }
        }
    }

    /// ⌘↓: a save panel named after the file, in Downloads; the copy is
    /// written where the person chose, and the controller removes its own.
    private func offerToSave(_ source: URL, named name: String, over window: NSWindow?) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = name
        panel.directoryURL = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
        panel.canCreateDirectories = true
        save(panel, window) { [done] response in
            if response == .OK, let destination = panel.url {
                let files = FileManager.default
                // The panel asked before replacing a file of that name.
                if files.fileExists(atPath: destination.path) {
                    try? files.removeItem(at: destination)
                }
                do {
                    try files.copyItem(at: source, to: destination)
                } catch {
                    NSSound.beep()
                }
            }
            done()
        }
    }

    // MARK: the panel's controller (the grid, in the responder chain)

    func begin(_ panel: QLPreviewPanel) {
        panel.dataSource = self
        panel.delegate = self
    }

    func end(_ panel: QLPreviewPanel) {
        panel.dataSource = nil
        panel.delegate = nil
        // Closed by the person -- Esc, the close button, Space in it -- the
        // controller is told; closed by the controller, `url` is gone.
        if url != nil {
            url = nil
            done()
        }
    }
}

extension FilePreview: QLPreviewPanelDataSource, QLPreviewPanelDelegate {
    public nonisolated func numberOfPreviewItems(in panel: QLPreviewPanel!) -> Int {
        MainActor.assumeIsolated { url == nil ? 0 : 1 }
    }

    public nonisolated func previewPanel(_ panel: QLPreviewPanel!, previewItemAt index: Int) -> QLPreviewItem! {
        MainActor.assumeIsolated { url as NSURL? }
    }
}
