import AppKit
import PostioFFI
import PostioKit
import QuickLookUI

/// The copy is the controller's, written to the app's temporary folder and removed when this says
/// the panel is gone (`focusSearchFileDone`). The panel gets only a file URL in that folder;
/// nothing is fetched and nothing leaves the machine.
@MainActor
public final class FilePreview: NSObject {
    private let done: () -> Void
    private let save: (NSSavePanel, NSWindow?, @escaping (NSApplication.ModalResponse) -> Void) -> Void

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

    private func offerToSave(_ source: URL, named name: String, over window: NSWindow?) {
        let panel = NSSavePanel()
        panel.nameFieldStringValue = name
        panel.directoryURL = FileManager.default.urls(for: .downloadsDirectory, in: .userDomainMask).first
        panel.canCreateDirectories = true
        save(panel, window) { [done] response in
            if response == .OK, let destination = panel.url {
                let files = FileManager.default
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

    func begin(_ panel: QLPreviewPanel) {
        panel.dataSource = self
        panel.delegate = self
    }

    func end(_ panel: QLPreviewPanel) {
        panel.dataSource = nil
        panel.delegate = nil
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
