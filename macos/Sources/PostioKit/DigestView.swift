import PostioFFI
import SwiftUI

public struct DigestWindowView<BodyView: View>: View {
    let model: DigestModel
    let column: CGFloat
    let binding: (String) -> String?
    let run: (String) -> Void
    let bodyView: (FocusReaderDocumentFfi, String?) -> BodyView

    public init(
        model: DigestModel, column: CGFloat,
        binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void,
        @ViewBuilder body: @escaping (FocusReaderDocumentFfi, String?) -> BodyView
    ) {
        self.model = model
        self.column = column
        self.binding = binding
        self.run = run
        bodyView = body
    }

    public var body: some View {
        VStack(spacing: 0) {
            if let view = model.view {
                switch view.page {
                case .email:
                    email(view)
                case .summary, .list:
                    DigestTabsBar(model: model, view: view)
                    Divider()
                    Group {
                        if view.page == .summary {
                            DigestSummaryPage(model: model, view: view, column: column)
                        } else {
                            DigestListPage(model: model, view: view)
                        }
                    }
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(MessageSurface.content)
                }
            } else {
                ProgressView().controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(MessageSurface.content)
            }
        }
        // The keyboard is the key monitor's: a focus ring on the first button would claim a focus
        // nobody moved there.
        .focusEffectDisabled()
    }

    @ViewBuilder
    private func email(_ view: DigestViewFfi) -> some View {
        VStack(spacing: 0) {
            if let banner = view.email?.banner {
                DigestCitationBanner(number: view.email?.number, text: banner)
                    .frame(width: column)
                    .padding(.vertical, 12)
                    .frame(maxWidth: .infinity)
                    .background(MessageSurface.chrome)
                    .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
            }
            if let header = model.emailView {
                MessageContentView(
                    words: MessageHeaderWords(view: header, binding: binding),
                    document: model.emailDocument,
                    binding: binding,
                    run: run,
                    alwaysForSender: {},
                    body: { document, _ in bodyView(document, model.highlight) })
                    .id(header.message)
            } else {
                ProgressView().controlSize(.small)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(MessageSurface.content)
    }
}

struct DigestTabsBar: View {
    let model: DigestModel
    let view: DigestViewFfi

    var body: some View {
        HStack(spacing: 10) {
            if view.tabs {
                HStack(spacing: 2) {
                    tab("Summary", on: view.page == .summary)
                    tab(view.listTab, on: view.page == .list)
                }
                .padding(2)
                .background(RoundedRectangle(cornerRadius: 7).fill(.quaternary))
                if let cap = model.tabCap { KeyCap(cap) }
            } else {
                Text(view.listTab).font(.system(size: 13, weight: .semibold))
            }
            Spacer(minLength: 12)
            let (said, button) = Self.split(view.ruleLine)
            if let said {
                Text(said).font(.system(size: 12.5)).foregroundStyle(.secondary).lineLimit(1)
            }
            FocusVerbButton(button, cap: model.ruleCap, action: model.editRule)
        }
        .padding(.horizontal, 16)
        .frame(height: 42)
        .background(MessageSurface.chrome)
    }

    private func tab(_ title: String, on: Bool) -> some View {
        Button(action: { if !on { model.toggleTab() } }) {
            Text(title)
                .font(.system(size: 13, weight: on ? .bold : .regular))
                .padding(.horizontal, 12)
                .frame(height: 26)
                .background {
                    if on {
                        RoundedRectangle(cornerRadius: 6).fill(MessageSurface.content)
                            .shadow(color: .black.opacity(0.08), radius: 1, y: 0.5)
                    }
                }
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    static func split(_ line: String) -> (String?, String) {
        guard let range = line.range(of: " \u{b7} ", options: .backwards) else { return (nil, line) }
        return (String(line[..<range.lowerBound]), String(line[range.upperBound...]))
    }
}

struct DigestSummaryPage: View {
    let model: DigestModel
    let view: DigestViewFfi
    let column: CGFloat

    /// The scheme a statement's click is carried on, inside this view only; never a link anywhere
    /// else.
    static let scheme = "postio-digest-reference"

    var body: some View {
        ScrollViewReader { scroller in
            ScrollView(.vertical) {
                VStack(alignment: .leading, spacing: 22) {
                    if view.loading && view.topics.isEmpty {
                        ProgressView().controlSize(.small)
                    }
                    ForEach(Array(view.topics.enumerated()), id: \.offset) { index, topic in
                        VStack(alignment: .leading, spacing: 6) {
                            heading(topic.heading)
                            paragraph(topic.statements)
                                .font(.system(size: 16))
                                .lineSpacing(6)
                                .fixedSize(horizontal: false, vertical: true)
                                .tint(.primary)
                                .environment(\.openURL, OpenURLAction { url in
                                    guard url.scheme == Self.scheme, let index = UInt32(url.host() ?? "")
                                    else { return .discarded }
                                    model.reference(index)
                                    return .handled
                                })
                            if model.focusedTopic == index, let card = view.card {
                                DigestCard(card: card, cap: model.cardCap, open: model.openReference)
                                    .padding(.top, 10)
                            }
                        }
                        .id(index)
                    }
                    if let footer = view.footer {
                        Divider()
                        Text(footer).font(.system(size: 12)).foregroundStyle(.tertiary)
                    }
                }
                .frame(width: column, alignment: .leading)
                .padding(.vertical, 28)
                .frame(maxWidth: .infinity)
            }
            .onChange(of: model.focusedTopic) { _, topic in
                guard let topic else { return }
                scroller.scrollTo(topic)
            }
        }
    }

    private func heading(_ text: String) -> some View {
        let parts = text.split(separator: " \u{b7} ", maxSplits: 1).map(String.init)
        return HStack(spacing: 6) {
            Text(parts.first ?? text).font(.system(size: 13.5, weight: .bold))
            if parts.count > 1 {
                Text(parts[1]).font(.system(size: 13)).foregroundStyle(.secondary)
            }
        }
        .accessibilityAddTraits(.isHeader)
    }

    private func paragraph(_ statements: [DigestStatementFfi]) -> Text {
        var text = AttributedString()
        for (position, statement) in statements.enumerated() {
            let focused = statement.index == view.focusedReference
            var words = AttributedString(statement.text)
            words.link = URL(string: "\(Self.scheme)://\(statement.index)")
            words.foregroundColor = .primary
            if focused { words.backgroundColor = Color.accentColor.opacity(0.12) }
            text += words
            var chip = AttributedString("\u{2009}\(statement.number)\u{2009}")
            chip.font = .system(size: 11, design: .monospaced)
            chip.foregroundColor = focused ? Color.accentColor : Color.secondary
            chip.backgroundColor = focused ? Color.accentColor.opacity(0.16) : Color.secondary.opacity(0.14)
            chip.link = URL(string: "\(Self.scheme)://\(statement.index)")
            text += AttributedString(" ")
            text += chip
            if position < statements.count - 1 { text += AttributedString(" ") }
        }
        return Text(text)
    }
}

struct DigestCard: View {
    let card: DigestCardFfi
    let cap: String?
    let open: () -> Void

    var body: some View {
        Button(action: open) {
            VStack(alignment: .leading, spacing: 6) {
                Text(card.title).font(.system(size: 13, weight: .semibold)).foregroundStyle(.primary)
                HStack(spacing: 5) {
                    if let cap { KeyCap(cap) }
                    Text(card.hint).font(.system(size: 12)).foregroundStyle(.secondary)
                }
            }
            .padding(12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color.accentColor.opacity(0.06)))
            .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(Color.accentColor, lineWidth: 1.5))
            .contentShape(RoundedRectangle(cornerRadius: 8))
        }
        .buttonStyle(.plain)
        .accessibilityLabel("\(card.title), \(card.hint)")
    }
}

struct DigestListPage: View {
    let model: DigestModel
    let view: DigestViewFfi

    var body: some View {
        ScrollViewReader { scroller in
            ScrollView(.vertical) {
                LazyVStack(spacing: 0) {
                    ForEach(Array(view.rows.enumerated()), id: \.offset) { index, row in
                        let focused = view.focused == UInt32(index)
                        VStack(alignment: .leading, spacing: 2) {
                            HStack {
                                Text(row.sender).font(.system(size: 13.5, weight: .semibold)).lineLimit(1)
                                Spacer()
                                Text(row.time).font(.system(size: 12).monospacedDigit()).foregroundStyle(.secondary)
                            }
                            Text(row.subject).font(.system(size: 13.5)).lineLimit(1)
                            if let preview = row.preview {
                                Text(preview).font(.system(size: 12.5)).foregroundStyle(.secondary).lineLimit(1)
                            }
                        }
                        .padding(.horizontal, 20)
                        .padding(.vertical, 10)
                        .background(focused ? Color.accentColor.opacity(0.08) : Color.clear)
                        .overlay {
                            if focused { Rectangle().strokeBorder(Color.accentColor, lineWidth: 2) }
                        }
                        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
                        .contentShape(Rectangle())
                        .onTapGesture(count: 2) { model.open(index) }
                        .simultaneousGesture(TapGesture().onEnded { model.point(index) })
                        .accessibilityElement(children: .combine)
                        .accessibilityAddTraits(focused ? .isSelected : [])
                        .id(index)
                    }
                }
            }
            .onChange(of: view.focused) { _, focused in
                guard let focused else { return }
                scroller.scrollTo(Int(focused))
            }
        }
    }
}

struct DigestCitationBanner: View {
    let number: UInt32?
    let text: String

    var body: some View {
        HStack(spacing: 8) {
            if let number {
                Text("\(number)")
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundStyle(Color.accentColor)
            }
            Text(text).font(.system(size: 13)).foregroundStyle(.secondary).lineLimit(2)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 7)
        .background(RoundedRectangle(cornerRadius: 6).fill(Color.accentColor.opacity(0.08)))
        .accessibilityElement(children: .combine)
    }
}

public struct DigestTitle: View {
    let model: DigestModel

    public init(model: DigestModel) {
        self.model = model
    }

    public var body: some View {
        VStack(spacing: 1) {
            HStack(spacing: 5) {
                if model.view?.page != .email {
                    Image(systemName: "square.stack.3d.up").font(.system(size: 11, weight: .semibold))
                }
                Text(model.view?.title ?? "")
                    .font(.system(size: 13.5, weight: .bold))
                    .lineLimit(1)
                    .truncationMode(.tail)
            }
            Text(model.view?.subtitle ?? "")
                .font(.system(size: 11.5))
                .foregroundStyle(.secondary)
                .lineLimit(1)
        }
        .frame(maxWidth: 420)
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isHeader)
    }
}

public struct DigestBack: View {
    let model: DigestModel

    public init(model: DigestModel) {
        self.model = model
    }

    public var body: some View {
        if let view = model.view, view.page == .email {
            Button(action: model.back) {
                HStack(spacing: 5) {
                    Image(systemName: "chevron.left").font(.system(size: 12, weight: .semibold))
                    Text(view.back).font(.system(size: 13.5, weight: .semibold))
                    if let cap = model.backCap { KeyCap(cap) }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(view.back)
        } else {
            Color.clear.frame(width: 1, height: 1)
        }
    }
}

public struct DigestTrailing: View {
    let model: DigestModel

    public init(model: DigestModel) {
        self.model = model
    }

    public var body: some View {
        if let view = model.view {
            if view.page == .email {
                HStack(spacing: 8) {
                    if let cap = model.stepCap { KeyCap(cap) }
                    HStack(spacing: 0) {
                        step("chevron.up", DigestCommand.previous, "Previous source")
                        Divider().frame(height: 18)
                        step("chevron.down", DigestCommand.next, "Next source")
                    }
                    .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(.separator))
                }
            } else {
                FocusDefaultButton(view.archive, cap: model.archiveCap, action: model.archiveAll)
            }
        } else {
            Color.clear.frame(width: 1, height: 1)
        }
    }

    private func step(_ symbol: String, _ command: String, _ label: String) -> some View {
        Button { model.step(command) } label: {
            Image(systemName: symbol).font(.system(size: 11, weight: .semibold))
                .frame(width: 28, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}
