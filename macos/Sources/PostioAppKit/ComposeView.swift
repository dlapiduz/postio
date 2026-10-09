import AppKit
import PostioFFI
import SwiftUI
import PostioKit

/// The composer (specs/009-focus-macos T079; screens 05 and 06), in the one
/// secondary window over the list (M4).
///
/// The rows are From (the account picker), To with the Cc and Bcc hint,
/// Cc and Bcc once asked for or holding somebody, and Subject; then the
/// body -- a reply's quote folded under it -- what is attached, and a
/// footer with Attach ⌘⇧A, Remind if no reply ⌘H (its day when on) and
/// the word count. The title area (`ComposeWindowChrome`) says what this
/// is and when it was saved, with Send later ▾ and Send ⌘↩.
///
/// Recipients complete from mail's correspondents and, after one prompt,
/// Contacts (`ContactsSource`): nothing shows until a recipient is typed,
/// ↑↓ choose, and Return or Tab take one.
///
/// Nothing here composes MIME, addresses a reply, or decides what a quote
/// looks like. It takes what the boundary handed over, shows it, and hands
/// back what was typed; when it is saved is the controller's.
public struct ComposeView: View {
    private let session: PostioSession
    private let model: ComposeModel
    private let accounts: [AccountFfi]
    private let contacts: ContactsSource?
    private let edited: () -> Void
    private let close: () -> Void

    @FocusState private var focus: Field?
    /// Whether the Link button is asking where to point.
    @State private var askingForLink = false
    @State private var confirmingDiscard = false
    /// The times the schedule-send picker is offering, or empty when it is
    /// closed. Held rather than computed in the dialog because they are
    /// computed *as it opens* -- see `schedulePresets`.
    @State private var schedule: [SchedulePresetFfi] = []
    /// The reminder's times, while ⌘H is asking which.
    @State private var reminders: [SchedulePresetFfi] = []
    @State private var linkAddress = ""

    private enum Field: Hashable {
        case to, cc, bcc, subject, body

        var recipient: RecipientField? {
            switch self {
            case .to: return .to
            case .cc: return .cc
            case .bcc: return .bcc
            default: return nil
            }
        }
    }

    /// The rows' rhythm, from screen 05.
    enum Metrics {
        static let rowHeight: CGFloat = 42
        static let labelWidth: CGFloat = 82
        static let inset: CGFloat = 20
        static let valueSize: CGFloat = 13.5
        static let labelSize: CGFloat = 13
        static let suggestionWidth: CGFloat = 440
        static let suggestionsShown = 6
    }

    public init(
        session: PostioSession,
        model: ComposeModel,
        accounts: [AccountFfi] = [],
        contacts: ContactsSource? = nil,
        edited: @escaping () -> Void,
        close: @escaping () -> Void
    ) {
        self.session = session
        self.model = model
        self.accounts = accounts
        self.contacts = contacts
        self.edited = edited
        self.close = close
    }

    public var body: some View {
        VStack(spacing: 0) {
            fromRow
            Divider()
            recipientRow("To", field: .to, text: Bindable(model).to, hint: !model.showsCopyFields)
                .zIndex(model.suggesting == .to ? 1 : 0)
            if model.showsCopyFields {
                Divider()
                recipientRow("Cc", field: .cc, text: Bindable(model).cc, hint: false)
                    .zIndex(model.suggesting == .cc ? 1 : 0)
                Divider()
                recipientRow("Bcc", field: .bcc, text: Bindable(model).bcc, hint: false)
                    .zIndex(model.suggesting == .bcc ? 1 : 0)
            }
            Divider()
            row("Subject") {
                TextField("", text: Bindable(model).subject)
                    .textFieldStyle(.plain)
                    .font(.system(size: Metrics.valueSize, weight: .semibold))
                    .focused($focus, equals: .subject)
                    .accessibilityLabel("Subject")
            }
            Divider()
            if model.rich { formatBar }
            bodyEditor
            if !model.attachments.isEmpty { attachments }
            if let status = model.status { statusRow(status) }
            Divider()
            footer
        }
        .background(Color(nsColor: .textBackgroundColor))
        .task {
            // A reply is addressed already: the keyboard goes to the body.
            // A moment after appearing, once the window is key: set before,
            // AppKit gave the window's first field (To) the keyboard over
            // it, its text selected.
            try? await Task.sleep(for: .milliseconds(60))
            focus = model.to.isEmpty ? .to : .body
        }
        .onAppear {
            // Which editor the hand-off names, read now: this window may
            // have been open since before the setting was chosen (#1288).
            model.refreshEditor()
        }
        // Coming back to this window is what a person means by "I am done
        // over there".
        .onReceive(
            NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)
        ) { _ in
            if model.isHandedOff { model.takeBack(through: session) }
        }
        // Autosave, because unsaved words are the thing a compose window must
        // never lose. On a pause rather than a keystroke: the controller
        // waits out the quiet period and says when (`FocusSaveDraft`), and
        // saves at once when the composer closes.
        .onChange(of: model.edited) { _, _ in edited() }
        // Completion follows typing in a recipient field, and only typing:
        // a reply opens with no list showing (screen 06).
        .onChange(of: model.to) { _, text in complete(.to, text) }
        .onChange(of: model.cc) { _, text in complete(.cc, text) }
        .onChange(of: model.bcc) { _, text in complete(.bcc, text) }
        .onChange(of: focus) { _, now in
            if now?.recipient != model.suggesting { model.dismissSuggestions() }
        }
        .modifier(Requests(
            model: model, session: session, close: close, schedule: $schedule,
            reminders: $reminders, askingForLink: $askingForLink, linkAddress: $linkAddress,
            confirmingDiscard: $confirmingDiscard, attach: attach, insertImage: insertImage))
    }

    // -- the rows -----------------------------------------------------------

    private func row<Value: View>(_ label: String, @ViewBuilder value: () -> Value) -> some View {
        HStack(spacing: 0) {
            Text(label)
                .font(.system(size: Metrics.labelSize))
                .foregroundStyle(.secondary)
                .frame(width: Metrics.labelWidth, alignment: .leading)
            value()
        }
        .padding(.horizontal, Metrics.inset)
        .frame(height: Metrics.rowHeight)
    }

    /// From: who it is written as, and the account picker on the right.
    private var fromRow: some View {
        row("From") {
            Text(fromName)
                .font(.system(size: Metrics.valueSize))
                .lineLimit(1)
            Spacer(minLength: 12)
            Menu {
                ForEach(accounts.filter(\.enabled), id: \.id) { account in
                    Button(account.address) { model.choose(account: account) }
                }
            } label: {
                Text(fromAddress)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .disabled(accounts.filter(\.enabled).count < 2)
            .accessibilityLabel("Write from")
        }
        .accessibilityElement(children: .contain)
    }

    /// The account the draft is written from, as the picker lists it.
    private var chosen: AccountFfi? { accounts.first { $0.id == model.account } }

    private var fromName: String {
        if let chosen, !chosen.displayName.isEmpty { return chosen.displayName }
        return model.from
    }

    private var fromAddress: String { chosen?.address ?? "" }

    /// A recipient field, with the list of what completes it hanging under
    /// it while it is being typed in.
    private func recipientRow(
        _ label: String, field: Field, text: Binding<String>, hint: Bool
    ) -> some View {
        row(label) {
            TextField("", text: text)
                .textFieldStyle(.plain)
                .font(.system(size: Metrics.valueSize))
                .focused($focus, equals: field)
                .accessibilityLabel(label)
                .onKeyPress(.downArrow) { suggestionKey { model.moveSuggestion(by: 1) } }
                .onKeyPress(.upArrow) { suggestionKey { model.moveSuggestion(by: -1) } }
                .onKeyPress(.return) { suggestionKey { model.acceptSuggestion() } }
                .onKeyPress(.tab) { suggestionKey { model.acceptSuggestion() } }
            if hint {
                Button { model.toggleCopyFields() } label: {
                    Text(copyHint)
                        .font(.system(size: 12))
                        .foregroundStyle(.secondary)
                }
                .buttonStyle(.plain)
                .accessibilityLabel("Show the Cc and Bcc fields")
            }
        }
        .overlay(alignment: .topLeading) {
            if model.suggesting == field.recipient, !model.suggestions.isEmpty {
                suggestionList
                    .offset(x: Metrics.inset + Metrics.labelWidth - 12, y: Metrics.rowHeight - 2)
            }
        }
    }

    /// A key the list answers while it is showing; the field's otherwise.
    private func suggestionKey(_ act: () -> Void) -> KeyPress.Result {
        guard model.suggesting != nil else { return .ignored }
        act()
        return .handled
    }

    /// "Cc · Bcc ⇧⌘C": both fields come with the one key (`copy_fields`),
    /// so the key is said once.
    private var copyHint: String {
        let cap = KeyCapSpelling.cap(session.binding(for: "copy_fields"))
        return cap.map { "Cc \u{b7} Bcc \($0)" } ?? "Cc \u{b7} Bcc"
    }

    /// ⌘H: the composer's own binding for the reminder, the chord rather
    /// than the list's bare `h`, which is a letter being typed here.
    private var remindCap: String? {
        session.bindings(for: "remind_if_no_reply")
            .first { $0.contains("+") }
            .flatMap { KeyCapSpelling.cap($0) }
    }

    /// What completes the words being typed: each a name and its address
    /// or a group, the highlighted one ringed.
    private var suggestionList: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(Array(model.suggestions.prefix(Metrics.suggestionsShown).enumerated()), id: \.offset) { index, suggestion in
                Button {
                    model.moveSuggestion(by: index - model.highlighted)
                    model.acceptSuggestion()
                } label: {
                    HStack {
                        Text(suggestion.label)
                            .font(.system(size: 13, weight: .semibold))
                            .lineLimit(1)
                        Spacer(minLength: 8)
                        if suggestion.group {
                            Text("group").font(.system(size: 12)).foregroundStyle(.secondary)
                        }
                    }
                    .padding(.horizontal, 8)
                    .frame(height: 30)
                    .contentShape(Rectangle())
                    .background(
                        RoundedRectangle(cornerRadius: 6)
                            .strokeBorder(Color.accentColor, lineWidth: index == model.highlighted ? 2 : 0)
                    )
                }
                .buttonStyle(.plain)
            }
        }
        .padding(6)
        .frame(width: Metrics.suggestionWidth, alignment: .leading)
        .background(
            RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor))
                .shadow(color: .black.opacity(0.18), radius: 10, y: 4)
        )
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Suggested recipients")
    }

    /// Ask what completes `text` in `field`, when the person is typing there.
    private func complete(_ field: RecipientField, _ text: String) {
        guard focus?.recipient == field else { return }
        guard !text.trimmingCharacters(in: .whitespaces).isEmpty else {
            model.dismissSuggestions()
            return
        }
        let session = session
        let account = model.account
        Task { @MainActor in
            let answer: [RecipientSuggestionFfi]
            if let contacts {
                answer = await contacts.suggestions(for: text) { words, extra in
                    session.recipientSuggestions(account: account, text: words, limit: 8, extra: extra)
                }
            } else {
                answer = await Task.detached {
                    session.recipientSuggestions(account: account, text: text, limit: 8, extra: [])
                }.value
            }
            model.suggest(answer, in: field, for: text)
        }
    }

    // -- the body -----------------------------------------------------------

    @ViewBuilder
    private var bodyEditor: some View {
        if model.rich {
            // A document, not a text field (#1271): the format bar's
            // marks have to apply to something, and on both frontends
            // that something is a contenteditable web view over
            // `postio_body`'s dialect.
            ComposeEditor(session: session, model: model)
                .disabled(model.isHandedOff)
                .accessibilityLabel("Message body")
        } else {
            VStack(alignment: .leading, spacing: 0) {
                TextEditor(text: Bindable(model).body)
                    .font(.system(size: 15))
                    .scrollContentBackground(.hidden)
                    .focused($focus, equals: .body)
                    .disabled(model.isHandedOff)
                    .accessibilityLabel("Message body")
                if let fold = model.quoteFold { quoteFold(fold) }
            }
            .padding(.horizontal, Metrics.inset - 5)
            .padding(.vertical, 14)
        }
    }

    /// The reply's quote, folded under what is written (screen 06):
    /// whose words, how many lines, and a way to show them.
    private func quoteFold(_ fold: QuoteFoldFfi) -> some View {
        Button { model.showQuote() } label: {
            HStack(spacing: 8) {
                Text("\u{203a} \(fold.summary)")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
                Text("show").font(.system(size: 13)).foregroundStyle(.secondary)
            }
            .padding(.horizontal, 10)
            .frame(height: 28)
            .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary.opacity(0.6)))
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .padding(.leading, 5)
        .padding(.bottom, 8)
        .accessibilityLabel("\(fold.summary). Show the quoted text")
    }

    /// What is attached, each with a way off again.
    private var attachments: some View {
        HStack(spacing: 8) {
            ForEach(model.attachments, id: \.id) { attachment in
                HStack(spacing: 8) {
                    Image(systemName: "doc")
                    VStack(alignment: .leading, spacing: 1) {
                        Text(attachment.filename)
                            .font(.system(size: 12.5, design: .monospaced))
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Text(attachment.size)
                            .font(.system(size: 11.5))
                            .foregroundStyle(.secondary)
                    }
                    Button {
                        model.detach(attachment, through: session)
                    } label: {
                        Image(systemName: "xmark")
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Remove \(attachment.filename)")
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
                .accessibilityElement(children: .contain)
            }
            Spacer()
        }
        .padding(.horizontal, Metrics.inset)
        .padding(.vertical, 10)
    }

    private func statusRow(_ status: String) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "exclamationmark.triangle")
            Text(status)
            Spacer()
        }
        .font(.callout)
        .padding(.horizontal, Metrics.inset)
        .padding(.vertical, 8)
        .background(.quaternary.opacity(0.5))
        .accessibilityElement(children: .combine)
    }

    // -- the format bar: rich drafts only ------------------------------------

    /// Only what mail actually renders: paragraph marks, lists, a quote and
    /// a link. Drawn while the draft is rich; the switch is in the footer.
    private var formatBar: some View {
        HStack(spacing: 8) {
            ForEach(ComposeFormat.marks, id: \.command) { mark in
                Button {
                    // The registry command *and* the document (#1271).
                    session.invoke(mark.command)
                    if mark.command == ComposeFormat.link {
                        linkAddress = ""
                        askingForLink = true
                    } else {
                        model.applyMark(mark.command)
                    }
                } label: {
                    Image(systemName: mark.symbol)
                        .foregroundStyle(
                            model.isMarkActive(mark.command)
                                ? Color(nsColor: PostioTokens.colorAccent) : Color.primary
                        )
                }
                .buttonStyle(.plain)
                .help(tooltip(mark.title, mark.command))
                .accessibilityLabel(mark.title)
                .disabled(!model.marksApply)
            }
            Spacer()
        }
        .padding(.horizontal, Metrics.inset)
        .frame(height: 32)
    }

    // -- the footer ---------------------------------------------------------

    /// Attach ⌘⇧A, Remind if no reply ⌘H (its day when on), and on the
    /// right the Rich/Plain switch and the word count.
    private var footer: some View {
        HStack(spacing: 6) {
            FocusVerbButton("Attach", cap: cap("attach_file")) { attach() }
            Menu {
                ForEach(remindPresets(), id: \.when) { preset in
                    Button(preset.label) { model.remindAt = preset.when }
                }
                if model.remindAt != nil {
                    Divider()
                    Button("Don\u{2019}t remind") { model.remindAt = nil }
                }
            } label: {
                Text(model.remindWords).font(.system(size: 13.5, weight: .semibold))
                .padding(.horizontal, 8)
                .frame(height: 28)
                .background(
                    RoundedRectangle(cornerRadius: 6)
                        .strokeBorder(.separator, lineWidth: model.remindAt == nil ? 0 : 1)
                )
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .accessibilityLabel(model.remindWords)
            // Beside the menu, not in its label: a menu's label is drawn
            // as text, and a keycap inside it is dropped.
            if let cap = remindCap { KeyCap(cap) }
            Spacer()
            if let recipients = model.recipientSummary {
                Text(recipients)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                    .accessibilityLabel("This message goes to \(recipients)")
                Text("\u{b7}").foregroundStyle(.secondary)
            }
            Picker("", selection: Bindable(model).rich) {
                Text("Rich").tag(true)
                Text("Plain").tag(false)
            }
            // Everything typed in Rich went into the document (#1293).
            .onChange(of: model.rich) { was, now in
                guard was, !now, let html = model.bodyHtml, !html.isEmpty else { return }
                model.switchedToPlain(text: session.plainTextOf(html))
            }
            .pickerStyle(.segmented)
            .controlSize(.small)
            .fixedSize()
            .disabled(model.isHandedOff)
            .help("Rich sends html and a plain-text alternative; Plain sends flowed text")
            .accessibilityLabel("How this message is written")
            Text(model.summary)
                .font(.system(size: 12.5))
                .foregroundStyle(.secondary)
                .padding(.leading, 4)
        }
        .padding(.horizontal, Metrics.inset - 8)
        .frame(height: 48)
        .background(Color(nsColor: .windowBackgroundColor))
    }

    // -- what the buttons do ------------------------------------------------

    private func attach() {
        // POSTIO-CONSENT: a file leaves this machine only because somebody
        // chose it in an open panel.
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = true
        panel.canChooseDirectories = false
        guard panel.runModal() == .OK else { return }
        model.attach(panel.urls, through: session)
    }

    private func insertImage() {
        // POSTIO-CONSENT: a picture leaves this machine only because somebody
        // chose it in an open panel, and it leaves inside the message they
        // are writing.
        let panel = NSOpenPanel()
        panel.allowsMultipleSelection = false
        panel.canChooseDirectories = false
        panel.allowedContentTypes = [.image]
        guard panel.runModal() == .OK, let file = panel.url else { return }
        model.insertImage(from: file, through: session)
    }

    private func cap(_ command: String) -> String? {
        KeyCapSpelling.cap(session.binding(for: command))
    }

    private func tooltip(_ title: String, _ command: String) -> String {
        guard let chord = session.accelerator(for: command) else { return title }
        return "\(title) (\(chord))"
    }
}

/// What a command asks of the composer and only a view can grant: a sheet,
/// an open panel, a confirmation, a list of times (`ComposeCommands` runs
/// with no view in reach; it records the wish, and this grants it).
private struct Requests: ViewModifier {
    let model: ComposeModel
    let session: PostioSession
    let close: () -> Void
    @Binding var schedule: [SchedulePresetFfi]
    @Binding var reminders: [SchedulePresetFfi]
    @Binding var askingForLink: Bool
    @Binding var linkAddress: String
    @Binding var confirmingDiscard: Bool
    let attach: () -> Void
    let insertImage: () -> Void

    func body(content: Content) -> some View {
        content
            .onChange(of: model.wantsLink) { _, wanted in
                guard wanted else { return }
                model.wantsLink = false
                linkAddress = ""
                askingForLink = true
            }
            .onChange(of: model.wantsAttachment) { _, wanted in
                guard wanted else { return }
                model.wantsAttachment = false
                attach()
            }
            .onChange(of: model.wantsImage) { _, wanted in
                guard wanted else { return }
                model.wantsImage = false
                insertImage()
            }
            .onChange(of: model.wantsSchedule) { _, wanted in
                guard wanted else { return }
                model.wantsSchedule = false
                schedule = schedulePresets()
            }
            .onChange(of: model.wantsRemind) { _, wanted in
                guard wanted else { return }
                model.wantsRemind = false
                reminders = remindPresets()
            }
            .confirmationDialog(
                "Send this message later?",
                isPresented: Binding(get: { !schedule.isEmpty }, set: { if !$0 { schedule = [] } }),
                titleVisibility: .visible
            ) {
                // The four times are the boundary's, recomputed as the
                // picker opens.
                ForEach(schedule, id: \.when) { preset in
                    Button(preset.label) {
                        model.send(at: preset.when, through: session)
                        schedule = []
                        if model.sent { close() }
                    }
                }
                Button("Cancel", role: .cancel) { schedule = [] }
            }
            .confirmationDialog(
                remindMeaning(at: nil),
                isPresented: Binding(get: { !reminders.isEmpty }, set: { if !$0 { reminders = [] } }),
                titleVisibility: .visible
            ) {
                ForEach(reminders, id: \.when) { preset in
                    Button(preset.label) {
                        model.remindAt = preset.when
                        reminders = []
                    }
                }
                if model.remindAt != nil {
                    Button("Don\u{2019}t remind") {
                        model.remindAt = nil
                        reminders = []
                    }
                }
                Button("Cancel", role: .cancel) { reminders = [] }
            }
            .onChange(of: model.wantsDiscard) { _, wanted in
                guard wanted else { return }
                model.wantsDiscard = false
                confirmingDiscard = true
            }
            // `Recovery::Confirm` in the registry, so the verb asks.
            .confirmationDialog(
                "Discard this draft?",
                isPresented: $confirmingDiscard,
                titleVisibility: .visible
            ) {
                Button("Discard", role: .destructive) {
                    model.discard(through: session)
                    close()
                }
                Button("Keep writing", role: .cancel) {}
            } message: {
                Text("What you have written will not be kept.")
            }
            .alert("Link to", isPresented: $askingForLink) {
                TextField("https://example.com", text: $linkAddress)
                Button("Link") { model.applyMark(ComposeFormat.link, href: linkAddress) }
                Button("Cancel", role: .cancel) {}
            } message: {
                Text("A message can link to http, https or mailto.")
            }
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
