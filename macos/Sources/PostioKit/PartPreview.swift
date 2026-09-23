import AppKit
import PDFKit
import SwiftUI

/// A part drawn inside Postio, from bytes that are already here.
///
/// The point of drawing it here rather than handing it to Preview.app is that
/// nothing in this view can fetch, launch, or tell the sender the attachment
/// was opened. `NSImage` and `PDFDocument` both take `Data`, so the bytes
/// never touch the filesystem either — there is no temporary file to leak and
/// none to clean up.
///
/// Only what `PartFfi.previewable` claims: images and PDFs. Anything else is
/// `PartOpening.desktop`, and this view never sees it.
struct PartPreview: View {
    let title: String
    let mimeType: String
    let bytes: Data
    let dismiss: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(title)
                    .font(.headline)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer()
                Button("Done", action: dismiss)
                    .keyboardShortcut(.cancelAction)
            }
            .padding(PostioTokens.space3)
            Divider()
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        }
        .frame(minWidth: 520, minHeight: 420)
    }

    @ViewBuilder private var content: some View {
        if mimeType == "application/pdf", let document = PDFDocument(data: bytes) {
            PDFPreview(document: document)
        } else if let image = NSImage(data: bytes) {
            Image(nsImage: image)
                .resizable()
                .scaledToFit()
                .padding(PostioTokens.space3)
                .accessibilityLabel(title)
        } else {
            // The type said it could be drawn and the bytes disagree. Say so
            // rather than showing an empty sheet: a blank pane looks like an
            // answer about the attachment, and this is an answer about the
            // file.
            Text("This \(mimeType) could not be drawn. Save it and open it elsewhere.")
                .foregroundStyle(.secondary)
                .padding(PostioTokens.space4)
        }
    }
}

/// PDFKit's view, which has no SwiftUI form of its own.
private struct PDFPreview: NSViewRepresentable {
    let document: PDFDocument

    func makeNSView(context _: Context) -> PDFView {
        let view = PDFView()
        view.autoScales = true
        view.document = document
        return view
    }

    func updateNSView(_ view: PDFView, context _: Context) {
        if view.document !== document { view.document = document }
    }
}
