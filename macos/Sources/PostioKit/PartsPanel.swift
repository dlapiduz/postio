import AppKit
import PostioFFI
import SwiftUI

/// A message's parts, and the four things you can do with one.
///
/// **Before this, an attachment that arrived on a Mac could not be saved.**
/// There was no parts surface and no other route to a message's MIME tree, so
/// a message with a PDF on it showed its body and nothing else.
///
/// The panel decides almost nothing. The tree, the box-drawing prefixes down
/// the left, what each row is called, what it says to a screen reader, what
/// its detail line reads, and the filename it is safe to write under all come
/// from the boundary — `postio-gtk` draws the same panel from the same
/// answers. What is local is the cursor and the dialogs, because only a view
/// can put up a save panel.
public struct PartsPanel: View {
    private let session: PostioSession
    private let message: Int64
    private let model: PartsModel
    private let dismiss: () -> Void
    /// What the reader held back for this message, so the notes can say so.
    /// Zeroes when nothing was.
    private let held: (remote: UInt32, trackers: UInt32)
    /// Render this message's held-back parts, once — see `RenderedOnce`.
    private let renderOnce: () -> Void

    @State private var failure: String?
    /// The part being drawn in a sheet over the panel, if any.
    @State private var previewing: (title: String, mime: String, bytes: Data)?

    public init(
        session: PostioSession,
        message: Int64,
        model: PartsModel,
        held: (remote: UInt32, trackers: UInt32) = (0, 0),
        renderOnce: @escaping () -> Void = {},
        dismiss: @escaping () -> Void
    ) {
        self.session = session
        self.message = message
        self.model = model
        self.held = held
        self.renderOnce = renderOnce
        self.dismiss = dismiss
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack {
                Text(model.summary)
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                Spacer()
                Button("Done", action: dismiss)
                    .keyboardShortcut(.cancelAction)
            }
            .padding(PostioTokens.space3)
            Divider()
            List(selection: Binding(get: { model.focused?.partId }, set: { picked in
                guard let picked,
                      let at = model.parts.firstIndex(where: { $0.partId == picked })
                else { return }
                model.put(cursor: UInt32(at))
            })) {
                ForEach(model.parts, id: \.partId) { part in
                    HStack(spacing: PostioTokens.space2) {
                        Text(part.prefix + part.label)
                            .font(.system(.body, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Spacer()
                        Text(part.detail)
                            .font(.system(.caption, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                    .tag(part.partId)
                    .accessibilityElement(children: .combine)
                    .accessibilityLabel(part.spoken)
                }
            }
            // The sentence beside the part the cursor is on — four states
            // with four different things to say, and this line is all a
            // reader gets when a `cid:` resolves to nothing and the body
            // draws a broken box (#751). A panel writing its own would be
            // the panel where that has no explanation.
            if let focused = model.focused {
                Divider()
                Text(partNote(part: focused, remoteImages: held.remote, trackers: held.trackers))
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.horizontal, PostioTokens.space3)
                    .padding(.vertical, PostioTokens.space2)
                // Offered only for the part that can actually load things:
                // an `image/png` references nothing and cannot phone home,
                // so offering to render *it* once would be theatre. The
                // boundary decides which, and says why above the button.
                if let reason = partHeldBackNote(
                    mimeType: focused.mimeType,
                    remoteImages: held.remote,
                    trackers: held.trackers
                ) {
                    HStack(spacing: PostioTokens.space2) {
                        Text(reason)
                            .font(.callout)
                            .foregroundStyle(.secondary)
                        Spacer(minLength: PostioTokens.space2)
                        // Not a grant: this asks for the document again with
                        // images allowed for this one message, and writes no
                        // allowlist entry — or a key meaning "just this
                        // once" would quietly mean "from now on".
                        Button("Render once") {
                            renderOnce()
                            dismiss()
                        }
                            .controlSize(.small)
                    }
                    .padding(.horizontal, PostioTokens.space3)
                    .padding(.bottom, PostioTokens.space2)
                }
            }
            if let failure {
                Divider()
                Text(failure)
                    .font(.callout)
                    .foregroundStyle(.red)
                    .padding(PostioTokens.space3)
            }
            Divider()
            HStack(spacing: PostioTokens.space2) {
                Button("Save…") { save() }
                    .disabled(!model.canSaveFocused)
                Button("Save all…") { saveAll() }
                Button("Open") { open() }
                    .disabled(!model.canSaveFocused)
                Spacer()
            }
            .padding(PostioTokens.space3)
        }
        .frame(minWidth: 420, minHeight: 260)
        .sheet(isPresented: Binding(get: { previewing != nil }, set: { if !$0 { previewing = nil } })) {
            if let previewing {
                PartPreview(
                    title: previewing.title,
                    mimeType: previewing.mime,
                    bytes: previewing.bytes,
                    dismiss: { self.previewing = nil }
                )
            }
        }
        // A command has no view, so it asks and the panel grants. Watching
        // the token rather than the wish: two saves in a row are two saves,
        // and `onChange` on the value alone would see only the first.
        .onChange(of: model.wishToken) { _, _ in
            switch model.wish {
            case .save: save()
            case .saveAll: saveAll()
            case .openExternally: openExternally()
            case .preview: open()
            case nil: break
            }
        }
    }

    /// Write the focused part where the user says.
    ///
    /// `saveName` in the name field, never `filename`: the second is the
    /// sender's own text and the first is what the boundary made safe.
    func save() {
        guard let part = model.focused else { return }
        let panel = NSSavePanel()
        panel.nameFieldStringValue = part.saveName
        guard panel.runModal() == .OK, let url = panel.url else { return }
        // The write happens in Rust, so the security scope has to be open on
        // this side or it fails as a permission error under App Sandbox.
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        attempt { try session.savePart(message, partId: part.partId, to: url.path) }
    }

    /// Write every savable part into one directory.
    func saveAll() {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.prompt = "Save all"
        guard panel.runModal() == .OK, let url = panel.url else { return }
        let scoped = url.startAccessingSecurityScopedResource()
        defer { if scoped { url.stopAccessingSecurityScopedResource() } }
        attempt {
            let saved = try session.saveAllParts(message, into: url.path)
            // One sentence for the batch, the boundary's wording, and only
            // when something actually failed.
            if let complaint = saved.failure { failure = complaint }
        }
    }

    /// Open the focused part — `Return`.
    ///
    /// What that means is `PartOpening`'s answer, which is the boundary's:
    /// something Postio can draw is drawn here, where a document cannot
    /// fetch or launch, and everything else goes to the desktop.
    func open() {
        guard let part = model.focused else { return }
        switch PartOpening.of(part) {
        case .preview:
            // Off the main actor: `partBytes` is the one call here that can
            // wait on a server. `PartOpening` has already established the
            // bytes are local, so this returns at once — but the rule is the
            // boundary's, not this view's to relax.
            failure = nil
            let session = session
            let message = message
            let part = part
            Task {
                do {
                    let bytes = try await Task.detached {
                        try session.partBytes(message, partId: part.partId)
                    }.value
                    previewing = (title: part.label, mime: part.mimeType, bytes: bytes)
                } catch let error as PartsError {
                    failure = switch error {
                    case let .Refused(message): message
                    }
                } catch {
                    failure = "\(error)"
                }
            }
        case .desktop:
            openExternally()
        case .nothing:
            break
        }
    }

    /// Hand the focused part to whichever application opens it.
    ///
    /// POSTIO-CONSENT: bytes leave Postio's own window only because somebody
    /// pressed this button. The file is written into a cache directory under
    /// a name Postio chooses — `exportPart` takes no filename, so a sender's
    /// `filename=` cannot reach the filesystem or the launcher.
    func openExternally() {
        guard let part = model.focused else { return }
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent("postio-parts", isDirectory: true)
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        attempt {
            let written = try session.exportPart(
                message, partId: part.partId, into: directory.path
            )
            NSWorkspace.shared.open(URL(fileURLWithPath: written))
        }
    }

    /// Run `work`, and put whatever it complains about on screen.
    ///
    /// A part that could not be written is the one case here that must not be
    /// silent: the user asked for a file and there is no file.
    private func attempt(_ work: () throws -> Void) {
        failure = nil
        do {
            try work()
        } catch let error as PartsError {
            failure = switch error {
            case let .Refused(message): message
            }
        } catch {
            failure = "\(error)"
        }
    }
}
