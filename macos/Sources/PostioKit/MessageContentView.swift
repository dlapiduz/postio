import PostioFFI
import SwiftUI

/// Numbers match `postio_ui::focus_dialog::rhythm`, which GTK's dialog spaces by. An absent block
/// takes the gap above it with it.
public enum MessageRhythm {
    public static let top: CGFloat = 28
    public static let bannerToKicker: CGFloat = 16
    public static let kickerToSubject: CGFloat = 6
    public static let markerToSubject: CGFloat = 12
    public static let subjectToLabels: CGFloat = 10
    public static let labelsToSender: CGFloat = 16
    public static let senderPadding: CGFloat = 12
    public static let senderRow: CGFloat = 22
    public static let senderLabelColumn: CGFloat = 44
    public static let senderLabelColumnReplying: CGFloat = 64
    public static let senderToCard: CGFloat = 12
    public static let cardToNotice: CGFloat = 12
    public static let cardToBody: CGFloat = 24
    public static let modeLineToBody: CGFloat = 12
    public static let bodyToAttachments: CGFloat = 24
    public static let attachmentsRuleToChips: CGFloat = 16
    public static let chipGap: CGFloat = 8
    public static let bottom: CGFloat = 32
    /// Column for a body in app colours until the engine reports one (`640 - 80`).
    public static let column: CGFloat = 560

    public static let subjectSize: CGFloat = 26
    public static let subjectLine: CGFloat = 32
    public static let pillHeight: CGFloat = 24
    public static let pillDot: CGFloat = 8
    public static let cardRadius: CGFloat = 8
    public static let cardMinHeight: CGFloat = 48
    public static let chipHeight: CGFloat = 40
    public static let chipRadius: CGFloat = 8
}

/// Recomputed at the window's current width, not the width it opened at.
public enum MessageColumn {
    public static func at(window: CGFloat, document: FocusReaderDocumentFfi) -> FocusMessageColumnFfi {
        guard window > 0 else {
            return FocusMessageColumnFfi(
                columnWidth: document.columnWidth, foldsIntoMore: document.foldsIntoMore)
        }
        return focusMessageColumn(windowWidth: Int32(window.rounded()), treatment: document.treatmentShown)
    }
}

public struct MessageHeaderWords: Equatable {
    public struct Key: Equatable {
        public let command: String
        public let label: String
        public let cap: String?
    }

    public struct Thread: Equatable, Sendable {
        public var kicker: String
        public var banner: ThreadBannerFfi?
        public var replyTo: String?
        public var newReply: ThreadBannerFfi?
        public var matchBanner: MatchBannerFfi?

        public static let none = Thread(kicker: "", banner: nil, replyTo: nil)
    }

    public struct Pill: Equatable {
        public let name: String
        public let colour: LabelColour?
    }

    public struct Card: Equatable {
        public let chip: String
        public let date: String?
        public let quoted: String?
        public let sentence: String?
        public let status: String?
        public let actions: [Key]
        public let dismiss: Key
    }

    public let thread: Thread
    public let subject: String
    public let labels: [Pill]
    public let addLabel: Key
    public let fields: [FocusFieldFfi]
    public let date: String
    public let card: Card?
    public let attachments: [FocusAttachmentFfi]

    public init(
        view: FocusMessageViewFfi, thread: Thread = .none, binding: (String) -> String?
    ) {
        func key(_ action: FocusRowActionFfi) -> Key {
            Key(command: action.command, label: action.label, cap: KeyCapSpelling.cap(binding(action.command)))
        }
        self.thread = thread
        subject = view.subject
        labels = view.labels.map { Pill(name: $0.name, colour: LabelColour.of(name: $0.name, stored: $0.color)) }
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
    var swatch: Color { Color(.sRGB, red: red, green: green, blue: blue) }
}

public struct MessageContentView<BodyView: View>: View {
    let words: MessageHeaderWords
    let document: FocusReaderDocumentFfi?
    let unsubscribe: Unsubscribing.State
    let binding: (String) -> String?
    let run: (String) -> Void
    let alwaysForSender: () -> Void
    let leave: () -> Void
    let clear: (Bool?) -> Void
    let openAttachment: (Int64) -> Void
    let bodyView: (FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView

    public init(
        words: MessageHeaderWords,
        document: FocusReaderDocumentFfi?,
        unsubscribe: Unsubscribing.State = .offered,
        binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void,
        alwaysForSender: @escaping () -> Void,
        leave: @escaping () -> Void = {},
        clear: @escaping (Bool?) -> Void = { _ in },
        openAttachment: @escaping (Int64) -> Void = { _ in },
        @ViewBuilder body: @escaping (FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView
    ) {
        self.words = words
        self.document = document
        self.unsubscribe = unsubscribe
        self.binding = binding
        self.run = run
        self.alwaysForSender = alwaysForSender
        self.leave = leave
        self.clear = clear
        self.openAttachment = openAttachment
        bodyView = body
    }

    private func column(_ width: CGFloat) -> CGFloat {
        document.map { CGFloat(MessageColumn.at(window: width, document: $0).columnWidth) }
            ?? MessageRhythm.column
    }

    public var body: some View {
        GeometryReader { proxy in
            scroll(column: column(proxy.size.width))
        }
        // Overlays the foot so nothing moves or resizes the text being read.
        .overlay(alignment: .bottom) {
            if let banner = MessageBlock.foot(words: words) {
                ThreadBannerView(banner: banner, tinted: true) { run(Self.latest) }
                    .padding(.horizontal, 16)
                    .padding(.bottom, 16)
            }
        }
    }

    private func scroll(column: CGFloat) -> some View {
        let blocks = MessageBlock.blocks(words: words, document: document, unsubscribe: unsubscribe)
        return ScrollView(.vertical) {
            VStack(alignment: .leading, spacing: 0) {
                ForEach(Array(blocks.enumerated()), id: \.element.id) { index, block in
                    draw(block)
                        .padding(
                            .top,
                            MessageBlock.gap(above: block, after: index > 0 ? blocks[index - 1] : nil))
                }
            }
            .frame(width: column, alignment: .leading)
            .padding(.top, MessageRhythm.top)
            .padding(.bottom, MessageRhythm.bottom)
            .frame(maxWidth: .infinity)
        }
    }

    private static var latest: String { "latest_in_thread" }

    @ViewBuilder
    private func draw(_ block: MessageBlock) -> some View {
        switch block {
        case let .olderBanner(banner):
            ThreadBannerView(banner: banner) { run(Self.latest) }
        case let .matchBanner(banner):
            MatchBannerView(banner: banner, next: { run("find_next") }, back: { run("back") })
        case let .kicker(kicker):
            Text(kicker)
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
        case let .subject(subject):
            Text(subject)
                .font(.system(size: MessageRhythm.subjectSize, weight: .bold))
                .lineSpacing(MessageRhythm.subjectLine - MessageRhythm.subjectSize - 5)
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .accessibilityAddTraits(.isHeader)
        case .labels:
            MessageLabelsLine(labels: words.labels, add: words.addLabel, run: run)
        case .sender:
            MessageSenderBlock(
                fields: words.fields, date: words.date, replyTo: words.thread.replyTo)
        case let .senderWarning(warning):
            Label(warning, systemImage: "exclamationmark.triangle")
                .font(.system(size: 12.5))
                .foregroundStyle(.orange)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        case let .card(card):
            MessageActionCard(card: card, run: run)
        case let .unsubscribe(offer, state):
            UnsubscribeBanner(
                offer: offer, state: state, leave: leave, clear: clear,
                clearCap: KeyCapSpelling.cap(binding("clear_list_mail")))
        case let .renderMode(mode):
            MessageRenderModeLine(
                line: RenderModeLine(mode, binding: binding), run: run, always: alwaysForSender)
        case .body:
            if let document { bodyView(document, words.card) }
        case let .caveat(caveat):
            Text(caveat)
                .font(.system(size: 12.5))
                .foregroundStyle(.tertiary)
        case .attachments:
            MessageAttachments(attachments: words.attachments, open: openAttachment)
        }
    }
}

/// The column draws `blocks` and nothing else, so a notice the boundary carries but the column
/// omits fails a test (#1585).
public enum MessageBlock: Equatable, Identifiable {
    case olderBanner(ThreadBannerFfi)
    case matchBanner(MatchBannerFfi)
    case kicker(String)
    case subject(String)
    case labels
    case sender
    case senderWarning(String)
    case card(MessageHeaderWords.Card)
    /// Gone once the list is left: its absence is the confirmation.
    case unsubscribe(UnsubscribeOfferFfi, Unsubscribing.State)
    case renderMode(RenderModeWordsFfi)
    case body
    case caveat(String)
    case attachments

    public var id: String {
        switch self {
        case .olderBanner: "olderBanner"
        case .matchBanner: "matchBanner"
        case .kicker: "kicker"
        case .subject: "subject"
        case .labels: "labels"
        case .sender: "sender"
        case .senderWarning: "senderWarning"
        case .card: "card"
        case .unsubscribe: "unsubscribe"
        case .renderMode: "renderMode"
        case .body: "body"
        case .caveat: "caveat"
        case .attachments: "attachments"
        }
    }

    public static func foot(words: MessageHeaderWords) -> ThreadBannerFfi? {
        words.thread.newReply
    }

    public static func blocks(
        words: MessageHeaderWords,
        document: FocusReaderDocumentFfi?,
        unsubscribe: Unsubscribing.State
    ) -> [MessageBlock] {
        var blocks: [MessageBlock] = []
        if let banner = words.thread.banner { blocks.append(.olderBanner(banner)) }
        if let banner = words.thread.matchBanner { blocks.append(.matchBanner(banner)) }
        if !words.thread.kicker.isEmpty { blocks.append(.kicker(words.thread.kicker)) }
        blocks += [.subject(words.subject), .labels, .sender]
        if let warning = document?.senderWarning { blocks.append(.senderWarning(warning)) }
        if let card = words.card { blocks.append(.card(card)) }
        if let document {
            if let offer = document.unsubscribe, unsubscribe != .left {
                blocks.append(.unsubscribe(offer, unsubscribe))
            }
            if let mode = document.renderMode { blocks.append(.renderMode(mode)) }
            blocks.append(.body)
            if let caveat = document.caveat { blocks.append(.caveat(caveat)) }
        }
        if !words.attachments.isEmpty { blocks.append(.attachments) }
        return blocks
    }

    public static func gap(above block: MessageBlock, after previous: MessageBlock?) -> CGFloat {
        switch block {
        case .olderBanner: previous == nil ? 0 : MessageRhythm.bannerToKicker
        case .matchBanner: previous == nil ? 0 : MessageRhythm.bannerToKicker
        case .kicker: previous == nil ? 0 : MessageRhythm.bannerToKicker
        case .subject:
            switch previous {
            case nil: 0
            case .kicker: MessageRhythm.kickerToSubject
            default: MessageRhythm.markerToSubject
            }
        case .labels: MessageRhythm.subjectToLabels
        case .sender: MessageRhythm.labelsToSender
        case .senderWarning: MessageRhythm.senderToCard
        case .card: MessageRhythm.senderToCard
        case .unsubscribe: MessageRhythm.cardToNotice
        case .renderMode: MessageRhythm.cardToBody
        case .body:
            if case .renderMode = previous { MessageRhythm.modeLineToBody } else { MessageRhythm.cardToBody }
        case .caveat: MessageRhythm.modeLineToBody
        case .attachments: MessageRhythm.bodyToAttachments
        }
    }
}

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

struct MessageSenderBlock: View {
    let fields: [FocusFieldFfi]
    let date: String
    /// E.g. "Ana Ruiz, Mon 21 Sep (message 1)"; nil when it answers the message just before.
    var replyTo: String?

    /// "Reply to" does not fit the usual column, so the block that has it widens it.
    private var labelColumn: CGFloat {
        replyTo == nil ? MessageRhythm.senderLabelColumn : MessageRhythm.senderLabelColumnReplying
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            ForEach(Array(fields.enumerated()), id: \.offset) { index, field in
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Text(field.field)
                        .font(.system(size: 13))
                        .foregroundStyle(.tertiary)
                        .frame(width: labelColumn, alignment: .leading)
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
            if let replyTo {
                HStack(alignment: .firstTextBaseline, spacing: 0) {
                    Text("Reply to")
                        .font(.system(size: 13))
                        .foregroundStyle(.tertiary)
                        .frame(width: labelColumn, alignment: .leading)
                    Text(replyTo)
                        .font(.system(size: 13.5))
                        .foregroundStyle(.primary)
                        .lineLimit(2)
                    Spacer(minLength: 12)
                }
                .frame(minHeight: MessageRhythm.senderRow)
            }
        }
        .padding(.vertical, MessageRhythm.senderPadding)
        .overlay(alignment: .top) { Rectangle().fill(.separator).frame(height: 1) }
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .accessibilityElement(children: .combine)
    }

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

struct MessageActionCard: View {
    let card: MessageHeaderWords.Card
    let run: (String) -> Void
    @Environment(\.colorScheme) private var scheme

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
                .fixedSize()
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
                    // First claim on the card's width, or the stack splits evenly and the sentence
                    // wraps beside empty space.
                    .layoutPriority(1)
            } else {
                Spacer(minLength: 0)
            }
            if let status = card.status {
                Text(status).font(.system(size: 13)).foregroundStyle(.secondary).fixedSize()
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
        .fixedSize()
    }
}

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

struct MessageAttachments: View {
    let attachments: [FocusAttachmentFfi]
    var open: (Int64) -> Void = { _ in }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Rectangle().fill(.separator).frame(height: 1)
            MessageFlow(spacing: MessageRhythm.chipGap) {
                ForEach(attachments, id: \.id) { part in
                    Button { open(part.id) } label: {
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
                    .buttonStyle(.plain)
                    .contentShape(Rectangle())
                }
            }
            .padding(.top, MessageRhythm.attachmentsRuleToChips)
        }
    }
}

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
