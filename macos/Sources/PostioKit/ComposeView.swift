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
    /// Whether the Link button is asking where to point.
    ///
    /// A mark that needs an argument cannot be a plain toggle: `insert_link`
    /// is the one entry in the bar that has to ask something before it can
    /// do anything, which is why it does not go through `markScript`.
    @State private var askingForLink = false
    @State private var linkAddress = ""

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
            if model.rich {
                // A document, not a text field (#1271): the format bar's
                // marks have to apply to something, and on both frontends
                // that something is a contenteditable web view over
                // `postio_body`'s dialect.
                ComposeEditor(session: session, model: model)
                    // Another editor holds it: two writers would each
                    // silently undo the other.
                    .disabled(model.isHandedOff)
                    .accessibilityLabel("Message body")
            } else {
                TextEditor(text: Bindable(model).body)
                    .font(.system(.body, design: .monospaced))
                    .focused($focus, equals: .body)
                    .disabled(model.isHandedOff)
                    .padding(PostioTokens.space3)
                    .accessibilityLabel("Message body")
            }
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
        // Coming back to this window is what a person means by "I am done
        // over there".
        .onReceive(
            NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            if model.isHandedOff { model.takeBack(through: session) }
        }
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
                    // The registry command *and* the document. `invoke`
                    // keeps this on the one path a keystroke takes -- undo
                    // included -- and `applyMark` is what reaches the
                    // surface the marks actually apply to (#1271).
                    session.invoke(mark.command)
                    if mark.command == ComposeFormat.link {
                        linkAddress = ""
                        askingForLink = true
                    } else {
                        model.applyMark(mark.command)
                    }
                } label: {
                    Image(systemName: mark.symbol)
                        // Lit when the caret is inside the mark, from the
                        // bridge's own reflection channel rather than from
                        // anything this window tracks -- a toolbar that
                        // guessed would be wrong the moment somebody moved
                        // the caret with the mouse.
                        .foregroundStyle(
                            model.isMarkActive(mark.command)
                                ? Color(nsColor: PostioTokens.colorAccent) : Color.primary
                        )
                }
                .help(tooltip(mark.title, mark.command))
                .accessibilityLabel(mark.title)
                .disabled(!model.marksApply)
            }
            Spacer()
            Picker("", selection: Bindable(model).rich) {
                Text("Rich").tag(true)
                Text("Plain").tag(false)
            }
            .pickerStyle(.segmented)
            .fixedSize()
            // Live since #1271. It was drawn disabled while the body was a
            // text field, because a switch over a body that cannot carry
            // marks would make the footer's claim about what leaves untrue.
            // The body carries marks now, and the footer follows the switch.
            .disabled(model.isHandedOff)
            .help("Rich sends html and a plain-text alternative; Plain sends flowed text")
            .accessibilityLabel("How this message is written")
        }
        .padding(.horizontal, PostioTokens.space4)
        .padding(.vertical, PostioTokens.space2)
        .alert("Link to", isPresented: $askingForLink) {
            TextField("https://example.com", text: $linkAddress)
            Button("Link") { model.applyMark(ComposeFormat.link, href: linkAddress) }
            Button("Cancel", role: .cancel) {}
        } message: {
            // Said before it is refused rather than after: the subset is
            // http, https and mailto, and a link to anything else would be
            // created, look right, and vanish at the next parse.
            Text("A message can link to http, https or mailto.")
        }
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
            // Named for what it does. `$EDITOR` is a shell variable an
            // application launched from Finder does not have (#1288), and a
            // button promising one would be promising the wrong thing.
            Button(model.isHandedOff ? "Take it back" : "Edit elsewhere") {
                if model.isHandedOff {
                    model.takeBack(through: session)
                } else {
                    model.handOff(through: session) { NSWorkspace.shared.open($0) }
                }
            }
            .buttonStyle(.link)
            .help(tooltip("Edit this draft in your text editor", "detach_composer"))
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

    /// The one mark that has to ask something before it can be applied.
    ///
    /// Named rather than written as a literal at the `if`, for the reason
    /// `Intercepted` names its commands: a literal that no longer matches
    /// the registry is a button that silently does nothing.
    public static let link = "insert_link"

    public static let marks: [Mark] = [
        Mark(command: "bold", symbol: "bold", title: "Bold"),
        Mark(command: "italic", symbol: "italic", title: "Italic"),
        Mark(command: "bullet_list", symbol: "list.bullet", title: "Bulleted list"),
        Mark(command: "numbered_list", symbol: "list.number", title: "Numbered list"),
        Mark(command: "quote_block", symbol: "text.quote", title: "Quote"),
        Mark(command: "insert_link", symbol: "link", title: "Link"),
    ]
}
