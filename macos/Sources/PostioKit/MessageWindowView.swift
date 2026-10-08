import PostioFFI
import SwiftUI

// The message window's chrome (specs/009-focus-macos T067; the Mac pack's
// message-window SPEC section 2): a 52pt title area -- the traffic lights,
// the subject and position line in the centre, one `k j` cap and the joined
// previous/next pair on the right -- then a 44pt action row with every verb
// and its key, Label, Move and Delete folding into More below 700.
//
// The title area is drawn into the window's unified toolbar
// (`MessageWindowChrome` in PostioAppKit), which is what gives it the
// 52pt height and centres the traffic lights in it; these are its views.

/// The chrome's words, from the engine's record and the bindings in force.
public struct MessageChromeWords: Equatable {
    /// One button: its command, its words, and its key as a cap.
    public struct Verb: Equatable, Identifiable {
        public let command: String
        public let label: String
        public let cap: String?
        public var id: String { command }
    }

    /// The subject.
    public let title: String
    /// "Message 5 of 60 · thread of 6".
    public let position: String
    /// The one cap beside the pair: previous then next, "k j".
    public let stepCap: String?
    /// The command the up chevron runs (`k`).
    public let previous = "prev_message"
    /// The command the down chevron runs (`j`).
    public let next = "next_message"
    /// The verbs the row draws, in order.
    public let row: [Verb]
    /// More, while anything waits in it.
    public let more: Verb?
    /// What waits in More.
    public let folded: [Verb]

    /// `view`'s chrome, in a window that `folds` (narrower than the fold,
    /// as the engine said), with keys spelled from `binding`.
    public init(view: FocusMessageViewFfi, folds: Bool, binding: (String) -> String?) {
        func verb(_ command: String, _ label: String) -> Verb {
            Verb(command: command, label: label, cap: KeyCapSpelling.cap(binding(command)))
        }
        title = view.subject
        position = view.position
        let caps = [previous, next].compactMap { KeyCapSpelling.cap(binding($0)) }
        stepCap = caps.isEmpty ? nil : caps.joined(separator: " ")
        let all = view.actions.map { (verb($0.command, $0.label), $0.folds) }
        let folding = folds && view.more != nil && all.contains { $0.1 }
        row = all.filter { !(folding && $0.1) }.map(\.0)
        folded = folding ? all.filter(\.1).map(\.0) : []
        more = folding ? view.more.map { verb($0.command, $0.label) } : nil
    }

    /// What the More menu lists: what folded, or every verb when nothing
    /// did (`.` still opens it).
    public var moreMenu: [Verb] { folded.isEmpty ? row : folded }
}

/// The title area's centre: the subject, 13.5 bold, and the position line
/// under it in the tertiary label colour.
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

/// The title area's right: one `k j` cap, then the previous/next pair --
/// two 30x28 icon buttons joined into one group, a 1px outline, radius 6,
/// and a 1px divider between them.
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
        .buttonStyle(.plain)
        .accessibilityLabel(label)
    }
}

/// The 44pt action row: every verb with its key, then More when anything
/// folded into it. A hairline under it.
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

    /// More's menu: one line per verb, with its key.
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
                .buttonStyle(.plain)
            }
        }
        .padding(6)
    }
}

/// A text button with its keycap: 13.5 medium, 30pt tall, padding 0 6, 5pt
/// between the words and the cap.
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
        .buttonStyle(.plain)
        .accessibilityLabel(verb.cap.map { "\(verb.label), key \($0)" } ?? verb.label)
    }
}

/// What the window holds under its title area: the action row, find while
/// it is open, and the content -- the message, or its raw source in place
/// of it (`v`, M4).
public struct MessageWindowView<BodyView: View>: View {
    let model: MessageWindowModel
    let binding: (String) -> String?
    let run: (String) -> Void
    let alwaysForSender: () -> Void
    let bodyView: (FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView

    public init(
        model: MessageWindowModel,
        binding: @escaping (String) -> String?,
        run: @escaping (String) -> Void,
        alwaysForSender: @escaping () -> Void,
        @ViewBuilder body: @escaping (FocusReaderDocumentFfi, MessageHeaderWords.Card?) -> BodyView
    ) {
        self.model = model
        self.binding = binding
        self.run = run
        self.alwaysForSender = alwaysForSender
        bodyView = body
    }

    public var body: some View {
        VStack(spacing: 0) {
            if let view = model.view {
                MessageActionRow(
                    words: MessageChromeWords(
                        view: view, folds: model.document?.foldsIntoMore ?? false, binding: binding),
                    moreOpen: model.moreOpen,
                    run: run,
                    setMore: { model.apply($0 ? .showMore : .closeMore) })
                    .background(MessageSurface.chrome)
            } else {
                Color.clear.frame(height: MessageActionRow.height)
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
        // The keyboard is the key monitor's, through the controller: a
        // focus ring on the first button would claim a focus nobody moved
        // there, and the accent is for the list's cursor (FR-017).
        .focusEffectDisabled()
    }

    @ViewBuilder
    private var content: some View {
        if model.showingSource {
            MessageSourceView(source: model.source)
        } else if let view = model.view {
            MessageContentView(
                words: MessageHeaderWords(view: view, binding: binding),
                document: model.document,
                binding: binding,
                run: run,
                alwaysForSender: alwaysForSender,
                body: bodyView)
                // A new message is read from its top; `O` keeps the place.
                .id(model.shown)
        } else {
            Color.clear
        }
    }
}

/// The raw source, in place of the content: every byte as sent, in mono,
/// selectable.
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

/// The surfaces the message window is painted with (SPEC section 6),
/// semantic so they follow light, dark and the system's own changes.
public enum MessageSurface {
    /// The content: `textBackgroundColor`.
    public static var content: Color {
        #if os(macOS)
            Color(nsColor: .textBackgroundColor)
        #else
            Color(uiColor: .systemBackground)
        #endif
    }

    /// The title area and the action row: the window's own background.
    public static var chrome: Color {
        #if os(macOS)
            Color(nsColor: .windowBackgroundColor)
        #else
            Color(uiColor: .secondarySystemBackground)
        #endif
    }
}

/// The title area's centre, reading the open message as it changes.
public struct MessageWindowTitle: View {
    let model: MessageWindowModel
    let binding: (String) -> String?

    public init(model: MessageWindowModel, binding: @escaping (String) -> String?) {
        self.model = model
        self.binding = binding
    }

    public var body: some View {
        if let view = model.view {
            MessageTitleView(words: MessageChromeWords(view: view, folds: false, binding: binding))
        }
    }
}

/// The title area's right, reading the open message as it changes.
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
                words: MessageChromeWords(view: view, folds: false, binding: binding), run: run)
        }
    }
}
