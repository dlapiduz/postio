import AppKit
import PostioFFI
import SwiftUI

/// The compose window (canvas screen 26).
///
/// Its own window, not a pane and not a sheet: writing a message is a thing
/// you do *beside* reading rather than instead of it, several at once, each
/// in the Window menu. The subject is the window's title, which is what makes
/// two of them tellable apart.
///
/// Nothing here composes MIME, addresses a reply, or decides what a quote
/// looks like. It takes what the boundary handed over, shows it, and hands
/// back what was typed.
public struct ComposeView: View {
    private let session: PostioSession
    private let model: ComposeModel
    private let close: () -> Void

    @FocusState private var focus: Field?

    private enum Field: Hashable {
        case to, cc, subject, body
    }

    public init(
        session: PostioSession,
        model: ComposeModel,
        close: @escaping () -> Void
    ) {
        self.session = session
        self.model = model
        self.close = close
    }

    public var body: some View {
        VStack(spacing: 0) {
            headers
            Divider()
            formatBar
            Divider()
            if !model.attachments.isEmpty { attachments }
            TextEditor(text: Bindable(model).body)
                .font(.system(.body, design: model.rich ? .default : .monospaced))
                .focused($focus, equals: .body)
                .padding(PostioTokens.space3)
                .accessibilityLabel("Message body")
            if let status = model.status {
                statusRow(status)
            }
            Divider()
            footer
        }
        .frame(minWidth: 520, minHeight: 420)
        .toolbar {
            ToolbarItem(placement: .primaryAction) {
                Button {
                    attach()
                } label: {
                    Image(systemName: "paperclip")
                }
                .help(tooltip("Attach a file", "attach_file"))
                .accessibilityLabel("Attach a file")
            }
            ToolbarItem(placement: .primaryAction) {
                Button(action: send) {
                    HStack(spacing: PostioTokens.space2) {
                        Text("Send")
                        if let chord = accelerator("send") {
                            Text(chord).opacity(0.75)
                        }
                    }
                }
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
                .help(tooltip("Send this message", "send"))
            }
        }
        .onAppear { focus = model.to.isEmpty ? .to : .body }
        // Autosave, because unsaved words are the thing a compose window must
        // never lose. On a pause rather than a keystroke: a save is one row,
        // but it is also one write lock, and typing is not the time to take
        // one.
        .onChange(of: model.edited) { _, _ in scheduleSave() }
        .onDisappear {
            if model.isDirty, !model.sent { model.save(through: session) }
            close()
        }
    }

    /// What is attached, each with a way off again.
    ///
    /// Above the body rather than below it: an attachment is part of what is
    /// being sent, and a list under the fold is one people forget they added.
    private var attachments: some View {
        HStack(spacing: PostioTokens.space2) {
            Text("Files")
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .frame(width: 72, alignment: .leading)
            ForEach(model.attachments, id: \.id) { attachment in
                HStack(spacing: PostioTokens.space2) {
                    Image(systemName: "doc")
                    Text(attachment.filename)
                        .lineLimit(1)
                        .truncationMode(.middle)
                    Text(attachment.size)
                        .font(.system(.caption, design: .monospaced))
                        .foregroundStyle(.secondary)
                    Button {
                        model.detach(attachment, through: session)
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Remove \(attachment.filename)")
                }
                .padding(.horizontal, PostioTokens.space2)
                .padding(.vertical, 3)
                .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: PostioTokens.radiusMd))
                .accessibilityElement(children: .contain)
            }
            Spacer()
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space2)
    }

    // -- the header fields --------------------------------------------------

    private var headers: some View {
        VStack(spacing: 0) {
            field("To", text: Bindable(model).to, focus: .to)
            Divider()
            field("From", value: model.draft.from)
            Divider()
            field("Subject", text: Bindable(model).subject, focus: .subject)
        }
    }

    private func field(
        _ label: String,
        text: Binding<String>,
        focus target: Field
    ) -> some View {
        HStack(spacing: PostioTokens.space4) {
            Text(label)
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .frame(width: 72, alignment: .leading)
            TextField("", text: text)
                .textFieldStyle(.plain)
                .focused($focus, equals: target)
                .accessibilityLabel(label)
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space3)
    }

    /// A field that is not edited — `From`, which is the account's identity
    /// rather than something to type into.
    private func field(_ label: String, value: String) -> some View {
        HStack(spacing: PostioTokens.space4) {
            Text(label)
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .frame(width: 72, alignment: .leading)
            Text(value)
            Spacer()
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space3)
        .accessibilityElement(children: .combine)
    }

    // -- the format bar -----------------------------------------------------

    /// Only what mail actually renders (canvas 26): paragraph marks, lists,
    /// a quote and a link. No fonts, no colours, no sizes — a mail client
    /// that offers them is one whose messages arrive looking like something
    /// else.
    private var formatBar: some View {
        HStack(spacing: PostioTokens.space2) {
            Text("Body")
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .frame(width: 72, alignment: .leading)
            ForEach(ComposeFormat.marks, id: \.command) { mark in
                Button {
                    session.invoke(mark.command)
                } label: {
                    Image(systemName: mark.symbol)
                }
                .help(tooltip(mark.title, mark.command))
                .accessibilityLabel(mark.title)
                .disabled(!model.rich)
            }
            Spacer()
            Picker("", selection: Bindable(model).rich) {
                Text("Rich").tag(true)
                Text("Plain").tag(false)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            // Drawn and disabled rather than removed: the canvas has this
            // control, and the honest state of it is "not yet". A live switch
            // over a body that cannot carry marks would make the footer's
            // claim about what leaves untrue, which is the one thing that
            // footer is for.
            .disabled(true)
            .help("Rich composition is not built yet — messages are sent as plain text")
            .accessibilityLabel("How this message is written")
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space2)
    }

    private func statusRow(_ status: String) -> some View {
        HStack(spacing: PostioTokens.space2) {
            Image(systemName: "exclamationmark.triangle")
            Text(status)
            Spacer()
        }
        .font(.callout)
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space2)
        .background(.quaternary.opacity(0.5))
        .accessibilityElement(children: .combine)
    }

    // -- the footer ---------------------------------------------------------

    private var footer: some View {
        HStack {
            Text(model.footer)
                .font(.system(.callout, design: .monospaced))
                .foregroundStyle(.secondary)
                .lineLimit(1)
                .truncationMode(.middle)
            Spacer()
            Button("Open in $EDITOR") { openInEditor() }
                .buttonStyle(.link)
                .help(tooltip("Edit this draft in your own editor", "detach_composer"))
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space2)
    }

    // -- what the buttons do ------------------------------------------------

    private func send() {
        if model.send(through: session) { close() }
    }

    private func attach() {
        // POSTIO-CONSENT: a file leaves this machine only because somebody
        // chose it in an open panel.
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK else { return }
        model.attach(panel.urls, through: session)
    }

    private func openInEditor() {
        model.save(through: session)
        model.handOff()
    }

    private func scheduleSave() {
        model.save(through: session)
    }

    private func accelerator(_ command: String) -> String? {
        session.accelerator(for: command)
    }

    private func tooltip(_ title: String, _ command: String) -> String {
        guard let chord = accelerator(command) else { return title }
        return "\(title) (\(chord))"
    }
}

/// The marks a message can carry, and nothing else.
///
/// Limited to what mail renders — paragraph style, bold, italic, underline,
/// monospace, lists, quote, link (canvas 26). The list is here rather than in
/// the view so it can be asserted, and every entry is a registry command.
public enum ComposeFormat {
    public struct Mark: Equatable, Sendable {
        public let command: String
        public let symbol: String
        public let title: String
    }

    public static let marks: [Mark] = [
        Mark(command: "bold", symbol: "bold", title: "Bold"),
        Mark(command: "italic", symbol: "italic", title: "Italic"),
        Mark(command: "bullet_list", symbol: "list.bullet", title: "Bulleted list"),
        Mark(command: "numbered_list", symbol: "list.number", title: "Numbered list"),
        Mark(command: "quote_block", symbol: "text.quote", title: "Quote"),
        Mark(command: "insert_link", symbol: "link", title: "Link"),
    ]
}
