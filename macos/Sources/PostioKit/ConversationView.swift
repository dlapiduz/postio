import PostioFFI
import SwiftUI

/// The conversation, stacked in the reading pane (#1263, ADR 0015 Q4).
///
/// Selecting a thread does not navigate anywhere: the list stays a list and
/// every message of the conversation appears here, oldest first. Read
/// messages are one line that never wraps; the latest and the unread ones are
/// open; a run of three or more collapsed messages is one divider standing in
/// for them.
///
/// Nothing here decides any of that. The fold arrives made
/// (`ConversationModel`), the divider's wording and its three-in-a-row
/// minimum are the boundary's, and every verb is a registry command rather
/// than something this view does itself — so `Reply` from the mouse and `e`
/// from the keyboard are one code path, undo included.
public struct ConversationView: View {
    private let session: PostioSession
    private let model: ConversationModel
    /// Run a command, and say which message the surface that ran it was
    /// drawn under.
    ///
    /// `nil` means the conversation as a whole — *Archive conversation* is
    /// about the thread, not about one of its messages. Everything else
    /// **must** name one: the per-message bar and the `⋯` menu ran `reply`
    /// and `forward` with no target at all, so they reached
    /// `Engine.replyDraft`, which falls back to the *list* cursor. In an
    /// eight-message thread, Reply under message three composed a reply to
    /// the thread's representative message — a wrong-recipient bug, with
    /// nothing on screen to say so.
    ///
    /// A parameter rather than a rule, so a new per-message surface cannot
    /// forget: the type will not let it.
    private let run: (String, Int64?) -> Void
    /// Whether a message is drawn as its sender wrote it, and how to change
    /// it. The engine holds this so `⌘O` can reach it.
    private let showingOriginal: (Int64) -> Bool
    /// Whether this message's held-back parts are rendered — see
    /// `RenderedOnce`, which holds it where `H` can reach it.
    private let showingImages: (Int64) -> Bool
    private let toggleOriginal: (Int64) -> Void

    public init(
        session: PostioSession,
        model: ConversationModel,
        run: @escaping (String, Int64?) -> Void,
        showingOriginal: @escaping (Int64) -> Bool,
        showingImages: @escaping (Int64) -> Bool,
        toggleOriginal: @escaping (Int64) -> Void
    ) {
        self.session = session
        self.model = model
        self.run = run
        self.showingOriginal = showingOriginal
        self.showingImages = showingImages
        self.toggleOriginal = toggleOriginal
    }

    /// What the stack draws, in order: messages, and dividers standing in for
    /// the runs of collapsed ones.
    private enum Entry: Identifiable {
        case message(Int)
        case folded(RunFfi)

        var id: String {
            switch self {
            case .message(let index): "m\(index)"
            case .folded(let run): "r\(run.start)"
            }
        }
    }

    /// The binding in force for `command`, drawn the way macOS draws it.
    ///
    private var entries: [Entry] {
        let runs = model.runs
        var entries: [Entry] = []
        var index = 0
        while index < model.rows.count {
            if let run = runs.first(where: { Int($0.start) == index }) {
                entries.append(.folded(run))
                index += Int(run.count)
            } else {
                entries.append(.message(index))
                index += 1
            }
        }
        return entries
    }

    public var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            ScrollView {
                LazyVStack(spacing: 0) {
                    ForEach(entries) { entry in
                        switch entry {
                        case .message(let index):
                            message(at: index)
                        case .folded(let run):
                            FoldedRun(run: run) { model.reveal(run) }
                        }
                    }
                }
            }
        }
        .accessibilityLabel(Pane.reader.label)
    }

    // -- the header ---------------------------------------------------------

    private var header: some View {
        VStack(alignment: .leading, spacing: PostioTokens.space2) {
            Text(model.subject)
                .font(.system(size: 22, weight: .semibold))
                .lineLimit(2)
                .accessibilityAddTraits(.isHeader)
            HStack(alignment: .firstTextBaseline) {
                Text(model.meta)
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                Spacer(minLength: PostioTokens.space4)
                Button {
                    model.expandAll()
                } label: {
                    HStack(spacing: PostioTokens.space2) {
                        Text("Expand all")
                        if let chord = session.accelerator(for: "expand_all") {
                            Text(chord)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .help("Open every message in this conversation")
                Menu {
                    // The thread, not a message in it -- which is what
                    // `archive_thread` means and what the other two act on
                    // through the list's own cursor.
                    Button("Archive conversation") { run("archive_thread", nil) }
                    Button("Mark unread") { run("mark_unread", nil) }
                    Button("Flag") { run("flag", nil) }
                } label: {
                    Image(systemName: "ellipsis")
                }
                .menuStyle(.borderlessButton)
                .fixedSize()
                .accessibilityLabel("More actions for this conversation")
            }
        }
        .padding(.horizontal, PostioTokens.space6)
        .padding(.vertical, PostioTokens.space4)
    }

    // -- one message --------------------------------------------------------

    @ViewBuilder
    private func message(at index: Int) -> some View {
        let row = model.rows[index]
        if model.expanded.indices.contains(index), model.expanded[index] {
            ExpandedMessage(
                session: session,
                row: row,
                isLatest: index == model.rows.count - 1,
                showingCc: model.isCcRevealed(index),
                showingOriginal: showingOriginal(row.id),
                showingImages: showingImages(row.id),
                collapse: { model.toggle(index) },
                toggleCc: { model.toggleCc(index) },
                toggleOriginal: { toggleOriginal(row.id) },
                run: run,
                openSettings: { run(Intercepted.settings, nil) }
            )
        } else {
            CollapsedMessage(row: row) { model.toggle(index) }
        }
    }
}

/// A read message: one line, never wrapping, one click from its body.
struct CollapsedMessage: View {
    let row: RowFfi
    let expand: () -> Void

    var body: some View {
        Button(action: expand) {
            HStack(spacing: PostioTokens.space3) {
                StateDot(seen: row.seen, mine: false)
                Text(row.from ?? "Unknown sender")
                    .fontWeight(row.seen ? .regular : .semibold)
                    .frame(width: 160, alignment: .leading)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Text(row.preview ?? "")
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: PostioTokens.space2)
                if row.hasAttachments {
                    Image(systemName: "paperclip")
                        .foregroundStyle(.secondary)
                        .accessibilityLabel("has an attachment")
                }
                Text(rowTimestamp(receivedAt: row.receivedAt))
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            .padding(.horizontal, PostioTokens.space6)
            .padding(.vertical, PostioTokens.space3)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(row.from ?? "Unknown sender"), \(row.preview ?? "")")
        .accessibilityHint("Expands this message")
        Divider()
    }
}

/// The divider that stands in for a run of collapsed messages.
struct FoldedRun: View {
    let run: RunFfi
    let show: () -> Void

    var body: some View {
        HStack(spacing: PostioTokens.space3) {
            rule
            Text(run.summary)
                .font(.callout)
                .foregroundStyle(.secondary)
                .lineLimit(1)
            Button("Show", action: show)
                .controlSize(.small)
            rule
        }
        .padding(.horizontal, PostioTokens.space6)
        .padding(.vertical, PostioTokens.space3)
        .accessibilityElement(children: .combine)
        .accessibilityLabel(run.summary)
        .accessibilityHint("Shows the messages this stands for")
        Divider()
    }

    private var rule: some View {
        Rectangle()
            .fill(Color.secondary.opacity(0.25))
            .frame(height: 1)
    }
}

/// An open message: who it is from, when it arrived, its body, and the three
/// things you can do about it.
public struct ExpandedMessage: View {
    public let session: PostioSession
    public let row: RowFfi
    public let isLatest: Bool
    /// Whether this message's `Cc` list is open.
    ///
    /// On the model rather than in `@State` here — as everything a person
    /// turned on in this pane now is: a disclosure is a thing somebody opened
    /// and can be asserted, and #1259 is what happens when a piece of the
    /// header exists only inside a view nobody can look at from a test.
    public let showingCc: Bool
    public let collapse: () -> Void
    public let toggleCc: () -> Void
    /// Run a command against **this** message. See `ConversationView.run`.
    public let run: (String, Int64?) -> Void
    public let openSettings: () -> Void

    @State private var height: CGFloat = BodyHeight.minimum
    /// Whether this message is drawn as its sender wrote it.
    ///
    /// Per message and per view: reader view is on by default for bulk mail
    /// and leaving it is one gesture about one message, not a mode.
    /// Whether this message is drawn as its sender wrote it.
    ///
    /// Passed in rather than `@State`, because `⌘O` is a command and a
    /// command cannot reach view state — which is why the key did nothing
    /// while the menu item below it worked. `OriginalView` holds it, per
    /// message and per view.
    public let showingOriginal: Bool
    /// Whether this *message's* images are showing.
    ///
    /// Per message and reset with the pane, which is what "show once" means:
    /// the standing grant is the popover's, and it is the only thing that
    /// survives closing the conversation.
    ///
    /// Held outside this view, like `showingOriginal` beside it and for the
    /// same reason: `H` (*Render part once*) is a command, and a command
    /// cannot reach an `@State`. While this was one, the notice's button
    /// worked and the key did nothing.
    public let showingImages: Bool
    /// Whether the `⋯` menu offers to fold this message away.
    ///
    /// `false` in the single-message pane, where there is no conversation to
    /// fold it into and the message would simply vanish. The rest of the
    /// header is the same on purpose — one implementation of "who is this
    /// from", not two that drift.
    public var collapsible = true
    public let toggleOriginal: () -> Void

    public init(
        session: PostioSession,
        row: RowFfi,
        isLatest: Bool,
        showingCc: Bool,
        showingOriginal: Bool,
        showingImages: Bool,
        collapsible: Bool = true,
        collapse: @escaping () -> Void,
        toggleCc: @escaping () -> Void,
        toggleOriginal: @escaping () -> Void,
        run: @escaping (String, Int64?) -> Void,
        openSettings: @escaping () -> Void
    ) {
        self.session = session
        self.row = row
        self.isLatest = isLatest
        self.showingCc = showingCc
        self.showingOriginal = showingOriginal
        self.showingImages = showingImages
        self.collapsible = collapsible
        self.collapse = collapse
        self.toggleCc = toggleCc
        self.toggleOriginal = toggleOriginal
        self.run = run
        self.openSettings = openSettings
    }

    public var body: some View {
        HStack(alignment: .top, spacing: 0) {
            Rectangle()
                .fill(Color(nsColor: PostioTokens.colorAccent))
                .frame(width: 3)
            VStack(alignment: .leading, spacing: PostioTokens.space3) {
                header
                // Per message, never per pane: a conversation can hold back
                // pictures from three senders and one notice above them all
                // could not say whose.
                if let notice = session.readerNotice(row.id), !notice.allowed, !showingImages {
                    BlockedImagesNotice(
                        notice: notice,
                        session: session,
                        show: { run(Intercepted.renderPartOnce, row.id) },
                        openSettings: openSettings
                    )
                }
                // Also per message: a conversation can hold eight messages
                // from four lists. `PRODUCT.md` lists one-click unsubscribe
                // among the privacy features, and until this existed the
                // sentence was not true on a Mac.
                if let offer = session.unsubscribeOffer(row.id) {
                    UnsubscribeBanner(offer: offer, message: row.id, session: session)
                }
                ReaderView(
                    session: session,
                    message: row.id,
                    remoteImages: remoteImages,
                    original: showingOriginal
                ) { measured in
                    height = measured
                }
                .frame(height: height)
                actions
            }
            .padding(.horizontal, PostioTokens.space4)
            .padding(.vertical, PostioTokens.space4)
        }
        .background(Color.primary.opacity(0.03))
        Divider()
    }

    /// Whether this message's remote images may load.
    ///
    /// `Blocked` unless somebody said otherwise about *this* message or
    /// about its sender. The default is the product's, not a convenience:
    /// fetching one picture tells the sender the message was opened, when,
    /// and roughly where from.
    private var remoteImages: RemoteImagesFfi {
        if showingImages { return .allowed }
        return session.readerNotice(row.id)?.allowed == true ? .allowed : .blocked
    }

    private var header: some View {
        HStack(alignment: .top, spacing: PostioTokens.space3) {
            Avatar(initials: row.initials)
            VStack(alignment: .leading, spacing: 2) {
                HStack(alignment: .firstTextBaseline, spacing: PostioTokens.space2) {
                    Text(row.from ?? "Unknown sender")
                        .fontWeight(.semibold)
                        .lineLimit(1)
                    if let address = row.fromAddress {
                        Text(address)
                            .font(.system(.callout, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                    }
                }
                Text(messageWhen(receivedAt: row.receivedAt))
                    .font(.system(.callout, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                recipients
            }
            Spacer(minLength: PostioTokens.space2)
            if isLatest {
                Text("latest")
                    .font(.system(.caption, design: .monospaced))
                    .padding(.horizontal, PostioTokens.space2)
                    .padding(.vertical, 2)
                    .overlay(
                        RoundedRectangle(cornerRadius: PostioTokens.radiusSm)
                            .stroke(Color(nsColor: PostioTokens.colorAccent), lineWidth: 1)
                    )
                    .foregroundStyle(Color(nsColor: PostioTokens.colorAccent))
            }
            Menu {
                if collapsible {
                    Button("Collapse", action: collapse)
                }
                Button("Reply") { run("reply", row.id) }
                Button("Forward") { run("forward", row.id) }
                Toggle(
                    "View original",
                    isOn: Binding(get: { showingOriginal }, set: { _ in toggleOriginal() })
                )
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .accessibilityLabel("More actions for this message")
        }
    }

    /// Who this message was addressed to (#1259).
    ///
    /// Every line already rendered by `postio_ui::reader::header`, which is
    /// what GTK's own reader draws from — so one recipient list reads the
    /// same on both. Read when the message is open rather than carried on
    /// the row: the list draws no recipients, and paying for them per row
    /// would load a mailbox's addresses to show one message's.
    ///
    /// Nothing at all when the message names nobody. A `To:` with no
    /// recipients after it is a line about the header rather than about the
    /// message.
    @ViewBuilder
    private var recipients: some View {
        if let lines = session.recipients(row.id) {
            if let to = lines.to {
                Text(to)
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
            }
            if let label = lines.ccLabel {
                // Folded until asked: the common message has one recipient,
                // and `Cc` costs nothing at all when there is none.
                Button(action: toggleCc) {
                    HStack(spacing: 2) {
                        Image(systemName: showingCc ? "chevron.down" : "chevron.right")
                        Text(label)
                    }
                }
                .buttonStyle(.borderless)
                .font(.callout)
                .foregroundStyle(.secondary)
                .accessibilityLabel(showingCc ? "Hide Cc recipients" : "Show Cc recipients")

                if showingCc, let cc = lines.cc {
                    Text(cc)
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    /// Reply, Reply All, Forward and Archive — the engine's list, in the
    /// engine's order.
    ///
    /// Built from `readerActions()` rather than written out here, so this bar
    /// and GTK's cannot end up offering different verbs. `archive` was the
    /// one missing: `a` archived and nothing on screen said so or offered
    /// another way, which is the keyboard-only gap #1221 closed for the list.
    private var actions: some View {
        HStack(spacing: PostioTokens.space3) {
            ForEach(
                ReaderActionPlan.items(
                    from: session.readerActions(),
                    available: { session.isAvailable($0, in: .reader) },
                    bindings: { session.bindings(for: $0) }
                ),
                id: \.command
            ) { item in
                action(item)
            }
            Spacer()
        }
    }

    @ViewBuilder
    private func action(_ item: ReaderActionPlan.Item) -> some View {
        let label = HStack(spacing: PostioTokens.space2) {
            Text(item.title)
            if let chord = item.chord {
                Text(chord).opacity(0.75)
            }
        }
        // Two branches rather than a style-erasing wrapper: `.buttonStyle`
        // takes a concrete type, and the ceremony of hiding that behind one
        // is longer than saying it twice.
        if item.prominent {
            Button(action: { run(item.command, row.id) }, label: { label })
                .buttonStyle(.borderedProminent)
                .disabled(!item.enabled)
        } else {
            Button(action: { run(item.command, row.id) }, label: { label })
                .buttonStyle(.bordered)
                .disabled(!item.enabled)
        }
    }

}

/// The square of initials beside a sender.
struct Avatar: View {
    let initials: String

    var body: some View {
        Text(initials)
            .font(.system(.caption, design: .monospaced))
            .frame(width: 28, height: 28)
            .background(
                RoundedRectangle(cornerRadius: PostioTokens.radiusSm)
                    .fill(Color(nsColor: PostioTokens.colorAccent).opacity(0.18))
            )
            .accessibilityHidden(true)
    }
}

/// Read, unread, or sent by me — the canvas's three dots.
struct StateDot: View {
    let seen: Bool
    let mine: Bool

    var body: some View {
        Group {
            if mine {
                RoundedRectangle(cornerRadius: 1)
                    .stroke(Color.secondary, lineWidth: 1)
            } else {
                RoundedRectangle(cornerRadius: 1)
                    .fill(seen ? Color.secondary.opacity(0.5) : Color(nsColor: PostioTokens.colorAccent))
            }
        }
        .frame(width: 8, height: 8)
        .accessibilityHidden(true)
    }
}
