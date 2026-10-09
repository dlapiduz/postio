import PostioFFI
import SwiftUI

// The message window's content column (specs/009-focus-macos T068; the Mac
// pack's message-window SPEC sections 3-5): the thread chip, the subject,
// the labels, the sender block, the action card, the render-mode line, the
// body and the attachments, all in one centred column as wide as the engine
// says for the treatment shown (M1, `columnWidth`).

/// The vertical rhythm (SPEC section 4: an 8pt grid around a 24pt body
/// line), in one place. The numbers are `postio_ui::focus_dialog::rhythm`'s,
/// which GTK's dialog spaces itself by. A block that is absent takes the
/// gap above it with it; the next gap stays as listed.
public enum MessageRhythm {
    /// The action row to the first block: the content's top padding.
    public static let top: CGFloat = 28
    /// The thread chip to the subject.
    public static let markerToSubject: CGFloat = 12
    /// The subject to the labels.
    public static let subjectToLabels: CGFloat = 10
    /// The labels to the sender block.
    public static let labelsToSender: CGFloat = 16
    /// Inside the sender block, above and below its rows.
    public static let senderPadding: CGFloat = 12
    /// One row of the sender block.
    public static let senderRow: CGFloat = 22
    /// The sender block's label column.
    public static let senderLabelColumn: CGFloat = 44
    /// The sender block to the action card.
    public static let senderToCard: CGFloat = 12
    /// The last block above the body to the render-mode line or the body.
    public static let cardToBody: CGFloat = 24
    /// The render-mode line to the body.
    public static let modeLineToBody: CGFloat = 12
    /// The body to the attachments' hairline.
    public static let bodyToAttachments: CGFloat = 24
    /// The attachments' hairline to the chips.
    public static let attachmentsRuleToChips: CGFloat = 16
    /// Between two attachment chips.
    public static let chipGap: CGFloat = 8
    /// Under the last block.
    public static let bottom: CGFloat = 32
    /// The column for a body in app colours, until the engine has said
    /// (`min(560, w - 80)` at any width the window opens at).
    public static let column: CGFloat = 560

    /// The subject: SF Pro bold 26/32, the largest text in the window.
    public static let subjectSize: CGFloat = 26
    public static let subjectLine: CGFloat = 32
    /// A label pill: 24pt, with an 8pt dot.
    public static let pillHeight: CGFloat = 24
    public static let pillDot: CGFloat = 8
    /// The action card: radius 8, at least 48 tall, padded 8/8/8/12.
    public static let cardRadius: CGFloat = 8
    public static let cardMinHeight: CGFloat = 48
    /// An attachment chip: 40 tall, radius 8.
    public static let chipHeight: CGFloat = 40
    public static let chipRadius: CGFloat = 8
}

/// The header block's words: the engine's, with keys spelled.
public struct MessageHeaderWords: Equatable {
    /// A button with its key.
    public struct Key: Equatable {
        public let command: String
        public let label: String
        public let cap: String?
    }

    /// "Latest of 6 in this thread  [ earlier message".
    public struct Thread: Equatable {
        public let text: String
        public let earlier: Key?
        public let later: Key?
    }

    /// A label pill.
    public struct Pill: Equatable {
        public let name: String
        public let colour: LabelColour?
    }

    /// The action card (SPEC section 5): the only filled element.
    public struct Card: Equatable {
        /// "To-do", "Question", "Invite".
        public let chip: String
        /// "Wed 30 Sep", in mono.
        public let date: String?
        /// The sentence, quoted, as the card draws it.
        public let quoted: String?
        /// The sentence itself, as the body highlights it.
        public let sentence: String?
        /// What stands where the actions would: "Accepted", "Past".
        public let status: String?
        /// The actions that answer it.
        public let actions: [Key]
        /// Dismiss `-`.
        public let dismiss: Key
    }

    public let thread: Thread?
    public let subject: String
    public let labels: [Pill]
    public let addLabel: Key
    public let fields: [FocusFieldFfi]
    public let date: String
    public let card: Card?
    public let attachments: [FocusAttachmentFfi]

    public init(view: FocusMessageViewFfi, binding: (String) -> String?) {
        func key(_ action: FocusRowActionFfi) -> Key {
            Key(command: action.command, label: action.label, cap: KeyCapSpelling.cap(binding(action.command)))
        }
        thread = view.thread.map {
            Thread(text: $0.text, earlier: $0.earlier.map(key), later: $0.later.map(key))
        }
        subject = view.subject
        labels = view.labels.map { Pill(name: $0.name, colour: LabelColour(hex: $0.color)) }
        addLabel = key(view.addLabel)
        fields = view.fields
        date = view.date
        card = view.marker.map { marker in
            Card(
                chip: marker.chip, date: marker.date,
                quoted: marker.quote.map { "\u{201c}\($0)\u{201d}" },
                sentence: marker.quote, status: marker.status,
                actions: marker.actions.map(key), dismiss: key(view.dismiss))
        }
        attachments = view.attachments
    }
}

/// The render-mode line's words (SPEC section 7): the treatment, why, the
/// switch with its key, and "Always for this sender" while it is offered.
public struct RenderModeLine: Equatable {
    public static let switchCommand = "switch_treatment"

    public let title: String
    public let detail: String
    public let action: MessageHeaderWords.Key
    public let always: String?

    public init(_ words: RenderModeWordsFfi, binding: (String) -> String?) {
        title = words.title
        detail = words.detail
        action = MessageHeaderWords.Key(
            command: Self.switchCommand, label: words.action,
            cap: KeyCapSpelling.cap(binding(Self.switchCommand)))
        always = words.offerAlways ? words.always : nil
    }
}

extension LabelColour {
    /// The label's own colour: data, not a design literal (SPEC: "Label
    /// dots are the only other hues").
    var swatch: Color { Color(.sRGB, red: red, green: green, blue: blue) }
}

/// The content column, scrolling under the window's fixed chrome.
public struct MessageContentView<BodyView: View>: View {
    let words: MessageHeaderWords
    let document: FocusReaderDocumentFfi?
    let binding: (String) -> String?
    let run: (String) -> Void
    let alwaysForSender: () -> Void
    let bodyView: (FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView

    public init(
        words: MessageHeaderWords,
        document: FocusReaderDocumentFfi?,
        binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void,
        alwaysForSender: @escaping () -> Void,
        @ViewBuilder body: @escaping (FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView
    ) {
        self.words = words
        self.document = document
        self.binding = binding
        self.run = run
        self.alwaysForSender = alwaysForSender
        bodyView = body
    }

    private var column: CGFloat {
        document.map { CGFloat($0.columnWidth) } ?? MessageRhythm.column
    }

    public var body: some View {
        ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 0) {
                if let thread = words.thread {
                    MessageThreadLine(thread: thread, run: run)
                        .padding(.bottom, MessageRhythm.markerToSubject)
                }
                Text(words.subject)
                    .font(.system(size: MessageRhythm.subjectSize, weight: .bold))
                    .lineSpacing(MessageRhythm.subjectLine - MessageRhythm.subjectSize - 5)
                    .foregroundStyle(.primary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
                    .accessibilityAddTraits(.isHeader)
                MessageLabelsLine(labels: words.labels, add: words.addLabel, run: run)
                    .padding(.top, MessageRhythm.subjectToLabels)
                MessageSenderBlock(fields: words.fields, date: words.date)
                    .padding(.top, MessageRhythm.labelsToSender)
                if let card = words.card {
                    MessageActionCard(card: card, run: run)
                        .padding(.top, MessageRhythm.senderToCard)
                }
                if let document {
                    if let mode = document.renderMode {
                        MessageRenderModeLine(
                            line: RenderModeLine(mode, binding: binding), run: run,
                            always: alwaysForSender)
                            .padding(.top, MessageRhythm.cardToBody)
                        bodyView(document, words.card)
                            .padding(.top, MessageRhythm.modeLineToBody)
                    } else {
                        bodyView(document, words.card)
                            .padding(.top, MessageRhythm.cardToBody)
                    }
                    if let caveat = document.caveat {
                        Text(caveat)
                            .font(.system(size: 12.5))
                            .foregroundStyle(.tertiary)
                            .padding(.top, MessageRhythm.modeLineToBody)
                    }
                }
                if !words.attachments.isEmpty {
                    MessageAttachments(attachments: words.attachments)
                        .padding(.top, MessageRhythm.bodyToAttachments)
                }
            }
            .frame(width: column, alignment: .leading)
            .padding(.top, MessageRhythm.top)
            .padding(.bottom, MessageRhythm.bottom)
            .frame(maxWidth: .infinity)
        }
    }
}

/// "Latest of 6 in this thread  [ earlier message".
struct MessageThreadLine: View {
    let thread: MessageHeaderWords.Thread
    let run: (String) -> Void

    var body: some View {
        HStack(spacing: 6) {
            Text(thread.text).foregroundStyle(.secondary)
            ForEach([thread.earlier, thread.later].compactMap { $0 }, id: \.command) { key in
                Button { run(key.command) } label: {
                    HStack(spacing: 5) {
                        if let cap = key.cap { KeyCap(cap) }
                        Text(key.label).foregroundStyle(.secondary)
                    }
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
            }
        }
        .font(.system(size: 13))
    }
}

/// The label pills, then "+ Label `l`".
struct MessageLabelsLine: View {
    let labels: [MessageHeaderWords.Pill]
    let add: MessageHeaderWords.Key
    let run: (String) -> Void

    var body: some View {
        HStack(spacing: 6) {
            ForEach(labels, id: \.name) { pill in
                HStack(spacing: 6) {
                    Circle()
                        .fill(pill.colour.map { AnyShapeStyle($0.swatch) } ?? AnyShapeStyle(.secondary))
                        .frame(width: MessageRhythm.pillDot, height: MessageRhythm.pillDot)
                    Text(pill.name).font(.system(size: 13)).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 8)
                .frame(height: MessageRhythm.pillHeight)
                .overlay(Capsule().strokeBorder(.separator, lineWidth: 1))
            }
            Button { run(add.command) } label: {
                HStack(spacing: 5) {
                    Text(add.label).font(.system(size: 13)).foregroundStyle(.secondary)
                    if let cap = add.cap { KeyCap(cap) }
                }
                .padding(.horizontal, 6)
                .frame(height: MessageRhythm.pillHeight)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
    }
}

/// The sender block: not a box, a hairline above and below; a 44pt label
/// column, the value (the name semibold, the address in mono), and the date
/// on the right in mono.
struct MessageSenderBlock: View {
    let fields: [FocusFieldFfi]
    let date: String

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(fields.enumerated()), id: \.offset) { index, field in
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Text(field.field)
                        .font(.system(size: 13))
                        .foregroundStyle(.tertiary)
                        .frame(width: MessageRhythm.senderLabelColumn, alignment: .leading)
                    people(field, first: index == 0)
                        .help(field.all)
                    Spacer(minLength: 12)
                    if index == 0 {
                        Text(date)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                            .fixedSize()
                    }
                }
                .frame(minHeight: MessageRhythm.senderRow)
            }
        }
        .padding(.vertical, MessageRhythm.senderPadding)
        .overlay(alignment: .top) { Rectangle().fill(.separator).frame(height: 1) }
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .accessibilityElement(children: .combine)
    }

    /// A field's people. The sender's name is semibold beside the address
    /// in mono; a recipient is their name, or the address alone in mono.
    private func people(_ field: FocusFieldFfi, first: Bool) -> some View {
        var line = Text("")
        for (index, person) in field.people.enumerated() {
            let comma = index + 1 < field.people.count || field.more != nil ? "," : ""
            if index > 0 { line = line + Text(" ") }
            if first, let name = person.name {
                line = line
                    + Text(name).font(.system(size: 13.5, weight: .semibold)).foregroundColor(.primary)
                    + Text(" ")
                    + Text(person.address).font(.system(size: 12.5, design: .monospaced))
                    .foregroundColor(.secondary)
            } else if let name = person.name {
                line = line + Text(name + comma).font(.system(size: 13.5)).foregroundColor(.primary)
            } else {
                line = line
                    + Text(person.address).font(.system(size: 12.5, design: .monospaced))
                    .foregroundColor(.primary)
                    + Text(comma).font(.system(size: 13.5)).foregroundColor(.primary)
            }
        }
        if let more = field.more {
            line = line + Text(" " + more).font(.system(size: 13.5)).foregroundColor(.secondary)
        }
        return line.lineLimit(2).textSelection(.enabled)
    }
}

/// The action card: the accent at 8% in light and 12% in dark (C26),
/// radius 8; an outlined kind tag, the date in mono, the sentence quoted in
/// italics, then its actions and Dismiss.
struct MessageActionCard: View {
    let card: MessageHeaderWords.Card
    let run: (String) -> Void
    @Environment(\.colorScheme) private var scheme

    /// C26: the accent's share of the card's fill.
    static func fill(_ scheme: ColorScheme) -> Double { scheme == .dark ? 0.12 : 0.08 }

    var body: some View {
        HStack(alignment: .center, spacing: 10) {
            Text(card.chip)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(Color.accentColor)
                .padding(.horizontal, 6)
                .frame(height: 20)
                .overlay(
                    RoundedRectangle(cornerRadius: 3).strokeBorder(Color.accentColor, lineWidth: 1))
            if let date = card.date {
                Text(date)
                    .font(.system(size: 12, design: .monospaced))
                    .foregroundStyle(Color.accentColor)
                    .fixedSize()
            }
            if let quoted = card.quoted {
                Text(quoted)
                    .font(.system(size: 13.5).italic())
                    .foregroundStyle(.primary)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Spacer(minLength: 0)
            }
            if let status = card.status {
                Text(status).font(.system(size: 13)).foregroundStyle(.secondary)
            }
            ForEach(card.actions, id: \.command) { key in
                button(key)
            }
            button(card.dismiss)
        }
        .padding(EdgeInsets(top: 8, leading: 12, bottom: 8, trailing: 8))
        .frame(minHeight: MessageRhythm.cardMinHeight)
        .background(
            RoundedRectangle(cornerRadius: MessageRhythm.cardRadius)
                .fill(Color.accentColor.opacity(Self.fill(scheme))))
        .accessibilityElement(children: .contain)
    }

    private func button(_ key: MessageHeaderWords.Key) -> some View {
        Button { run(key.command) } label: {
            HStack(spacing: 5) {
                Text(key.label).font(.system(size: 13.5, weight: .medium)).foregroundStyle(.primary)
                if let cap = key.cap { KeyCap(cap) }
            }
            .padding(.horizontal, 6)
            .frame(height: 28)
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
    }
}

/// The quiet line above an HTML body (12.5pt, tertiary): the treatment,
/// why, "Always for this sender" while offered, and the switch with `O`.
struct MessageRenderModeLine: View {
    let line: RenderModeLine
    let run: (String) -> Void
    let always: () -> Void

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: "circle.lefthalf.filled")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            Text(line.title).foregroundStyle(.secondary).fontWeight(.medium)
            Text(line.detail).foregroundStyle(.tertiary)
            Spacer(minLength: 8)
            if let words = line.always {
                Button(action: always) { Text(words).foregroundStyle(.secondary) }
                    .buttonStyle(.plain)
            }
            Button { run(line.action.command) } label: {
                HStack(spacing: 5) {
                    Text(line.action.label).foregroundStyle(.secondary)
                    if let cap = line.action.cap { KeyCap(cap) }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
        }
        .font(.system(size: 12.5))
        .lineLimit(1)
    }
}

/// The attachments: a hairline, then the chips, 8 apart, wrapping.
struct MessageAttachments: View {
    let attachments: [FocusAttachmentFfi]

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Rectangle().fill(.separator).frame(height: 1)
            MessageFlow(spacing: MessageRhythm.chipGap) {
                ForEach(attachments, id: \.id) { part in
                    HStack(spacing: 8) {
                        Image(systemName: "doc")
                            .font(.system(size: 14))
                            .foregroundStyle(.secondary)
                        Text(part.name)
                            .font(.system(size: 13.5))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Text(part.size)
                            .font(.system(size: 11.5, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 12)
                    .frame(height: MessageRhythm.chipHeight)
                    .overlay(
                        RoundedRectangle(cornerRadius: MessageRhythm.chipRadius)
                            .strokeBorder(.separator, lineWidth: 1))
                    .accessibilityElement(children: .combine)
                }
            }
            .padding(.top, MessageRhythm.attachmentsRuleToChips)
        }
    }
}

/// Lays its children out in rows, wrapping at the proposed width.
struct MessageFlow: Layout {
    let spacing: CGFloat

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()) -> CGSize {
        let width = proposal.width ?? .infinity
        var x: CGFloat = 0
        var y: CGFloat = 0
        var line: CGFloat = 0
        var widest: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > 0, x + size.width > width {
                y += line + spacing
                x = 0
                line = 0
            }
            x += size.width + spacing
            line = max(line, size.height)
            widest = max(widest, x - spacing)
        }
        return CGSize(width: min(widest, width), height: y + line)
    }

    func placeSubviews(
        in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache _: inout ()
    ) {
        var x = bounds.minX
        var y = bounds.minY
        var line: CGFloat = 0
        for subview in subviews {
            let size = subview.sizeThatFits(.unspecified)
            if x > bounds.minX, x + size.width > bounds.maxX {
                y += line + spacing
                x = bounds.minX
                line = 0
            }
            subview.place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(size))
            x += size.width + spacing
            line = max(line, size.height)
        }
    }
}
