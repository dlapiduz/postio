import PostioFFI
import SwiftUI

// The title area is drawn into the window's unified toolbar (`MessageWindowChrome` in
// PostioAppKit), which gives it the 52pt height; these are its views.

public struct MessageChromeWords: Equatable {
    public struct Verb: Equatable, Identifiable {
        public let command: String
        public let label: String
        public let cap: String?
        public var id: String { command }
    }

    public let title: String
    public let position: String
    public let stepCap: String?
    public let previous = "prev_message"
    public let next = "next_message"
    public let row: [Verb]
    public let more: Verb?
    public let folded: [Verb]

    public init(
        view: FocusMessageViewFfi, folds: Bool, place: String? = nil, binding: (String) -> String?
    ) {
        func verb(_ command: String, _ label: String) -> Verb {
            Verb(command: command, label: label, cap: KeyCapSpelling.cap(binding(command)))
        }
        title = view.subject
        position = place ?? view.position
        let caps = [previous, next].compactMap { KeyCapSpelling.cap(binding($0)) }
        stepCap = caps.isEmpty ? nil : caps.joined(separator: " ")
        let all = view.actions.map { (verb($0.command, $0.label), $0.folds) }
        let folding = folds && view.more != nil && all.contains { $0.1 }
        row = all.filter { !(folding && $0.1) }.map(\.0)
        folded = folding ? all.filter(\.1).map(\.0) : []
        more = folding ? view.more.map { verb($0.command, $0.label) } : nil
    }

    public var moreMenu: [Verb] { folded.isEmpty ? row : folded }
}

public struct MessageTitleView: View {
    let words: MessageChromeWords

    public init(words: MessageChromeWords) {
        self.words = words
    }

    public var body: some View {
        VStack(spacing: 1) {
            Text(words.title)
                .font(.system(size: 13.5, weight: .bold))
                .foregroundStyle(.primary)
                .lineLimit(1)
                .truncationMode(.tail)
            Text(words.position)
                .font(.system(size: 11.5))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
        }
        .frame(maxWidth: 420)
        .accessibilityElement(children: .combine)
    }
}

public struct MessageStepperView: View {
    public static let button = CGSize(width: 30, height: 28)
    public static let radius: CGFloat = 6

    let words: MessageChromeWords
    let run: (String) -> Void

    public init(words: MessageChromeWords, run: @escaping (String) -> Void) {
        self.words = words
        self.run = run
    }

    public var body: some View {
        HStack(spacing: 8) {
            if let cap = words.stepCap { KeyCap(cap) }
            HStack(spacing: 0) {
                chevron("chevron.up", words.previous, "Previous message")
                Rectangle().fill(.separator).frame(width: 1, height: Self.button.height)
                chevron("chevron.down", words.next, "Next message")
            }
            .clipShape(RoundedRectangle(cornerRadius: Self.radius))
            .overlay(
                RoundedRectangle(cornerRadius: Self.radius).strokeBorder(.separator, lineWidth: 1))
        }
        .fixedSize()
    }

    private func chevron(_ symbol: String, _ command: String, _ label: String) -> some View {
        Button { run(command) } label: {
            Image(systemName: symbol)
                .font(.system(size: 12, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: Self.button.width, height: Self.button.height)
                .contentShape(Rectangle())
        }
        // Square: the group's outline rounds the pair's outer corners.
        .buttonStyle(ChromeButtonStyle(radius: 0))
        .accessibilityLabel(label)
    }
}

public struct MessageActionRow: View {
    public static let height: CGFloat = 44
    public static let buttonHeight: CGFloat = 30

    let words: MessageChromeWords
    let moreOpen: Bool
    let run: (String) -> Void
    let setMore: (Bool) -> Void

    public init(
        words: MessageChromeWords, moreOpen: Bool, run: @escaping (String) -> Void,
        setMore: @escaping (Bool) -> Void
    ) {
        self.words = words
        self.moreOpen = moreOpen
        self.run = run
        self.setMore = setMore
    }

    public var body: some View {
        HStack(spacing: 0) {
            ForEach(words.row) { verb in
                MessageVerbButton(verb: verb) { run(verb.command) }
            }
            if let more = words.more {
                MessageVerbButton(verb: more) { setMore(true) }
                    .popover(isPresented: moreBinding, arrowEdge: .bottom) { menu }
            }
            Spacer(minLength: 0)
                .popover(isPresented: words.more == nil ? moreBinding : .constant(false)) { menu }
        }
        .padding(.horizontal, 12)
        .frame(height: Self.height)
        .frame(maxWidth: .infinity)
        .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
        .accessibilityElement(children: .contain)
        .accessibilityLabel("Message actions")
    }

    private var moreBinding: Binding<Bool> {
        Binding(get: { moreOpen }, set: { setMore($0) })
    }

    private var menu: some View {
        VStack(alignment: .leading, spacing: 2) {
            ForEach(words.moreMenu) { verb in
                Button {
                    setMore(false)
                    run(verb.command)
                } label: {
                    HStack(spacing: 5) {
                        Text(verb.label).font(.system(size: 13.5))
                        Spacer(minLength: 16)
                        if let cap = verb.cap { KeyCap(cap) }
                    }
                    .padding(.horizontal, 8)
                    .frame(minWidth: 160, minHeight: 26)
                    .contentShape(Rectangle())
                }
                .buttonStyle(ChromeButtonStyle())
            }
        }
        .padding(6)
    }
}

public struct MessageVerbButton: View {
    let verb: MessageChromeWords.Verb
    let action: () -> Void

    public init(verb: MessageChromeWords.Verb, action: @escaping () -> Void) {
        self.verb = verb
        self.action = action
    }

    public var body: some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Text(verb.label).font(.system(size: 13.5, weight: .medium))
                if let cap = verb.cap { KeyCap(cap) }
            }
            .foregroundStyle(.primary)
            .padding(.horizontal, 6)
            .frame(height: MessageActionRow.buttonHeight)
            .contentShape(Rectangle())
        }
        .buttonStyle(ChromeButtonStyle())
        .accessibilityLabel(verb.cap.map { "\(verb.label), key \($0)" } ?? verb.label)
        .background(
            GeometryReader { proxy in
                Color.clear.preference(
                    key: MessageVerbFrames.self,
                    value: [verb.command: proxy.frame(in: .named(MessageVerbFrames.space))])
            })
    }
}

/// Keyed by command, in the message window's content (top-left origin).
public struct MessageVerbFrames: PreferenceKey {
    public static let space = "postio.message.content"
    public static let defaultValue: [String: CGRect] = [:]

    public static func reduce(value: inout [String: CGRect], nextValue: () -> [String: CGRect]) {
        value.merge(nextValue()) { $1 }
    }
}

public struct MessageWindowView<BodyView: View>: View {
    let model: MessageWindowModel
    let binding: (String) -> String?
    let run: (String) -> Void
    let alwaysForSender: () -> Void
    let verbFrames: ([String: CGRect]) -> Void
    let barRoom: (UInt32) -> Void
    let barChip: (UInt32) -> Void
    let barMore: (UInt32) -> Void
    let bodyView: (Int64, FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView

    /// `verbFrames` hears where the action buttons are whenever they move; `barRoom` how many
    /// thread-bar chips fit, on open and every resize.
    public init(
        model: MessageWindowModel,
        binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void,
        alwaysForSender: @escaping () -> Void,
        verbFrames: @escaping ([String: CGRect]) -> Void = { _ in },
        barRoom: @escaping (UInt32) -> Void = { _ in },
        barChip: @escaping (UInt32) -> Void = { _ in },
        barMore: @escaping (UInt32) -> Void = { _ in },
        @ViewBuilder body: @escaping (Int64, FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView
    ) {
        self.model = model
        self.binding = binding
        self.run = run
        self.alwaysForSender = alwaysForSender
        self.verbFrames = verbFrames
        self.barRoom = barRoom
        self.barChip = barChip
        self.barMore = barMore
        bodyView = body
    }

    public var body: some View {
        GeometryReader { proxy in
            window(width: proxy.size.width)
                .onChange(of: ThreadBarMetrics.room(window: proxy.size.width), initial: true) {
                    barRoom($1)
                }
        }
    }

    private func folds(_ width: CGFloat) -> Bool {
        model.document.map { MessageColumn.at(window: width, document: $0).foldsIntoMore } ?? false
    }

    private func window(width: CGFloat) -> some View {
        VStack(spacing: 0) {
            if let view = model.view {
                MessageActionRow(
                    words: MessageChromeWords(view: view, folds: folds(width), place: model.walkPlace, binding: binding),
                    moreOpen: model.moreOpen,
                    run: run,
                    setMore: { model.apply($0 ? .showMore : .closeMore) })
                    .background(MessageSurface.chrome)
            } else {
                Color.clear.frame(height: MessageActionRow.height)
            }
            if let bar = model.bar {
                ThreadBarView(bar: bar, menu: { run(ThreadMenuCommand.menu) }, show: barChip, more: barMore)
                    .background(MessageSurface.chrome)
            }
            if model.find.isOpen {
                FindBar(
                    find: model.find,
                    setQuery: { model.setFindQuery($0) },
                    next: { model.apply(.findNext) },
                    previous: { model.apply(.findPrevious) },
                    close: { model.apply(.closeFind) })
                    .overlay(alignment: .bottom) { Rectangle().fill(.separator).frame(height: 1) }
            }
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(MessageSurface.content)
        }
        // The key monitor owns the keyboard: a focus ring on the first button would claim focus
        // nobody moved there.
        .focusEffectDisabled()
        .overlay(alignment: .bottom) {
            if let notice = model.notice {
                Text(notice)
                    .font(.system(size: 12.5))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 8))
                    .padding(.bottom, 16)
                    .accessibilityAddTraits(.updatesFrequently)
            }
        }
        .coordinateSpace(name: MessageVerbFrames.space)
        .onPreferenceChange(MessageVerbFrames.self) { frames in verbFrames(frames) }
    }

    @ViewBuilder
    private var content: some View {
        if model.showingSource {
            MessageSourceView(source: model.source)
        } else if let view = model.view, let displayed = model.displayed {
            // One `ForEach` by message keeps the out-of-sight body's web view when it comes up, not
            // made again blank. A new message is read from its top; `O` keeps the place.
            ZStack(alignment: .top) {
                ForEach(layers(displayed: displayed, view: view), id: \.message) { layer in
                    let front = layer.message == displayed
                    MessageContentView(
                        words: MessageHeaderWords(
                            view: layer.view,
                            thread: .init(
                                kicker: model.kicker, banner: model.olderBanner,
                                replyTo: model.replyTo, newReply: model.newReply,
                                matchBanner: model.matchBanner),
                            binding: binding),
                        document: layer.document,
                        unsubscribe: model.unsubscribeState(of: layer.message),
                        binding: binding,
                        run: run,
                        alwaysForSender: alwaysForSender,
                        leave: { run(Intercepted.unsubscribe) },
                        clear: { model.clearListMail(delete: $0) },
                        openAttachment: { model.openAttachment($0) },
                        body: { document, card in bodyView(layer.message, document, card) })
                        .environment(\.messageBodyDrawn) { model.bodyDrawn(layer.message) }
                        .opacity(front ? 1 : 0)
                        .zIndex(front ? 1 : 0)
                        .allowsHitTesting(front)
                        .accessibilityHidden(!front)
                }
            }
        } else {
            Color.clear
        }
    }
}

extension MessageWindowView {
    struct Layer {
        let message: Int64
        let view: FocusMessageViewFfi
        let document: FocusReaderDocumentFfi?
    }

    func layers(displayed: Int64, view: FocusMessageViewFfi) -> [Layer] {
        var layers = [Layer(message: displayed, view: view, document: model.document)]
        if let next = model.incoming, next.message != displayed {
            layers.append(Layer(message: next.message, view: next.view, document: next.document))
        }
        return layers
    }
}

/// Said by a body once it has its height and has painted, so one drawn out of sight comes up whole.
private struct MessageBodyDrawnKey: EnvironmentKey {
    static let defaultValue: @MainActor @Sendable () -> Void = {}
}

extension EnvironmentValues {
    public var messageBodyDrawn: @MainActor @Sendable () -> Void {
        get { self[MessageBodyDrawnKey.self] }
        set { self[MessageBodyDrawnKey.self] = newValue }
    }
}

public struct MessageSourceView: View {
    let source: String?

    public init(source: String?) {
        self.source = source
    }

    public var body: some View {
        ScrollView([.vertical, .horizontal]) {
            Text(source ?? "")
                .font(.system(size: 12, design: .monospaced))
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .topLeading)
                .padding(24)
        }
        .accessibilityLabel("Raw source")
    }
}

public enum MessageSurface {
    public static var content: Color {
        #if os(macOS)
            Color(nsColor: .textBackgroundColor)
        #else
            Color(uiColor: .systemBackground)
        #endif
    }

    public static var chrome: Color {
        #if os(macOS)
            Color(nsColor: .windowBackgroundColor)
        #else
            Color(uiColor: .secondarySystemBackground)
        #endif
    }
}

public struct MessageWindowTitle: View {
    let model: MessageWindowModel
    let binding: (String) -> String?

    public init(model: MessageWindowModel, binding: @escaping (String) -> String?) {
        self.model = model
        self.binding = binding
    }

    public var body: some View {
        if let view = model.view {
            MessageTitleView(words: MessageChromeWords(view: view, folds: false, place: model.walkPlace, binding: binding))
        }
    }
}

public struct MessageWindowStepper: View {
    let model: MessageWindowModel
    let binding: (String) -> String?
    let run: (String) -> Void

    public init(
        model: MessageWindowModel, binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void
    ) {
        self.model = model
        self.binding = binding
        self.run = run
    }

    public var body: some View {
        if let view = model.view {
            MessageStepperView(
                words: MessageChromeWords(view: view, folds: false, place: model.walkPlace, binding: binding), run: run)
        }
    }
}
