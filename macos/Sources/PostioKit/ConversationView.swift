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
    private let run: (String) -> Void

    public init(
        session: PostioSession,
        model: ConversationModel,
        run: @escaping (String) -> Void
    ) {
        self.session = session
        self.model = model
        self.run = run
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
    /// `MenuPlan.accelerator` rather than a second renderer: the chord beside
    /// `Reply` here and the one in the Message menu are the same key, and two
    /// spellings of it would be two things to get wrong when somebody rebinds.
    private func accelerator(for command: String) -> String? {
        session.binding(for: command).flatMap(MenuPlan.accelerator(from:))
    }

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
                        if let chord = accelerator(for: "expand_all") {
                            Text(chord)
                                .foregroundStyle(.secondary)
                        }
                    }
                }
                .help("Open every message in this conversation")
                Menu {
                    Button("Archive conversation") { run("archive_thread") }
                    Button("Mark unread") { run("mark_unread") }
                    Button("Flag") { run("flag") }
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
                collapse: { model.toggle(index) },
                run: run,
                openSettings: { run(Intercepted.settings) }
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
struct ExpandedMessage: View {
    let session: PostioSession
    let row: RowFfi
    let isLatest: Bool
    let collapse: () -> Void
    let run: (String) -> Void
    let openSettings: () -> Void

    @State private var height: CGFloat = BodyHeight.minimum
    /// Whether this *message's* images are showing.
    ///
    /// Per message and reset with the pane, which is what "show once" means:
    /// the standing grant is the popover's, and it is the only thing that
    /// survives closing the conversation.
    @State private var showingImages = false

    var body: some View {
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
                        show: { showingImages = true },
                        openSettings: openSettings
                    )
                }
                ReaderView(
                    session: session,
                    message: row.id,
                    remoteImages: remoteImages
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
                Button("Collapse", action: collapse)
                Button("Reply") { run("reply") }
                Button("Forward") { run("forward") }
                Button("View original") { run("view_original") }
            } label: {
                Image(systemName: "ellipsis")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
            .accessibilityLabel("More actions for this message")
        }
    }

    private var actions: some View {
        HStack(spacing: PostioTokens.space3) {
            action("Reply", command: "reply", prominent: true)
            action("Reply All", command: "reply_all", prominent: false)
            action("Forward", command: "forward", prominent: false)
            Spacer()
        }
    }

    @ViewBuilder
    private func action(_ title: String, command: String, prominent: Bool) -> some View {
        let label = HStack(spacing: PostioTokens.space2) {
            Text(title)
            if let chord = accelerator(for: command) {
                Text(chord).opacity(0.75)
            }
        }
        // Two branches rather than a style-erasing wrapper: `.buttonStyle`
        // takes a concrete type, and the ceremony of hiding that behind one
        // is longer than saying it twice.
        if prominent {
            Button(action: { run(command) }, label: { label })
                .buttonStyle(.borderedProminent)
                .disabled(!session.isAvailable(command, in: .reader))
        } else {
            Button(action: { run(command) }, label: { label })
                .buttonStyle(.bordered)
                .disabled(!session.isAvailable(command, in: .reader))
        }
    }

    /// The binding in force for `command`, drawn the way macOS draws it.
    private func accelerator(for command: String) -> String? {
        session.binding(for: command).flatMap(MenuPlan.accelerator(from:))
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
