import Foundation
import PostioFFI

/// The conversation the reading pane is showing, and what a reader has done
/// to it since it opened (#1263, ADR 0015 Q4).
///
/// **It decides as little as possible.** Where a conversation opens and which
/// messages are expanded when it does are `postio_ui::conversation`'s answers,
/// arriving through the boundary already made, and the page itself is the
/// boundary's too (ADR 0032, #1595).
///
/// What is genuinely local is what a person then does -- move to the next
/// message, fold this one, open them all -- and since the page is one
/// document with its script off, those are requests *to the page*
/// (`documentRequest`) rather than a model it is drawn from.
///
/// No AppKit here, deliberately (#1264): a conversation is the same thing on
/// a phone, and the only reason this could not go there is if the logic came
/// wrapped in a Mac view.
@MainActor
@Observable
public final class ConversationModel {
    /// The conversation as the boundary handed it over, or `nil` before one
    /// has been asked for.
    public private(set) var conversation: ConversationFfi?

    /// One flag per message: whether its body is open.
    public private(set) var expanded: [Bool] = []

    /// The message the keyboard is on inside the conversation.
    ///
    /// Starts where the boundary said the pane opens — the first unread —
    /// and `J`/`K` walk it from there. Separate from the *list's* cursor on
    /// purpose: `j` moves between conversations and `J` moves inside one,
    /// which is `PRODUCT.md` §9's distinction one level down.
    public private(set) var focused: Int = 0

    /// Something the conversation document has been asked to do (#1595).
    ///
    /// The pane is one page (ADR 0032), and what a person folds, opens or
    /// scrolls to is that page's state -- with the page's own script off, the
    /// application cannot even see a summary being clicked. So the keys do not
    /// change a model the page is drawn from; they ask the page, and the
    /// request is what can be asserted.
    public enum DocumentAction: Equatable, Sendable {
        /// Bring this message to the top of the pane -- `J`, `K`, a rail
        /// row. `settle` is the rail's token for the scroll, handed back
        /// through `settled` once the page is there.
        case scrollTo(message: Int64, settle: UInt64?)
        /// Fold or unfold this message -- `z`.
        case toggle(message: Int64)
        /// Open every message -- *Expand all*.
        case expandAll
    }

    /// One request, and which one it is, so the same request twice is two.
    public struct DocumentRequest: Equatable, Sendable {
        public let action: DocumentAction
        public let serial: Int

        public init(action: DocumentAction, serial: Int) {
            self.action = action
            self.serial = serial
        }

        /// The message it is about, if it is about one.
        public var message: Int64? {
            switch action {
            case let .scrollTo(message, _), let .toggle(message): message
            case .expandAll: nil
            }
        }

        /// Whether it takes the pane somewhere.
        public var isScroll: Bool {
            if case .scrollTo = action { true } else { false }
        }

        /// The rail's token for a scroll, if this is one it is waiting on.
        public var settle: UInt64? {
            if case let .scrollTo(_, settle) = action { settle } else { nil }
        }
    }

    /// The rail's state: which message is marked, and whether the observer
    /// is being listened to (#1576). `postio_ui::reader::rail::Rail`, held
    /// behind the boundary, so a chosen row, `J`/`K` and the observer all
    /// move the mark through one place and can never disagree.
    private let rail = RailFfi(count: 0)

    /// Which message the rail has marked -- the one the reader is on.
    public private(set) var marked: Int?

    /// The most recent request, or `nil` since the conversation opened.
    public private(set) var documentRequest: DocumentRequest?
    private var requestsAsked = 0

    private func ask(_ action: DocumentAction) {
        requestsAsked += 1
        documentRequest = DocumentRequest(action: action, serial: requestsAsked)
    }

    /// The message the keyboard is on, if there is one.
    private var focusedMessage: Int64? {
        rows.indices.contains(focused) ? rows[focused].id : nil
    }

    public init() {}

    /// The messages of the conversation, oldest first.
    public var rows: [RowFfi] { conversation?.rows ?? [] }

    /// Which message the pane opens on — the first unread, else the newest.
    public var focus: UInt32? { conversation?.focus }

    /// The subject, and the line under it.
    public var subject: String { conversation?.subject ?? "" }
    public var meta: String { conversation?.meta ?? "" }

    /// Show `conversation`, forgetting everything about the last one.
    ///
    /// A pane that kept its expansions across a selection would be drawing
    /// one conversation's state over another's, which looks like an answer.
    public func show(_ conversation: ConversationFfi) {
        self.conversation = conversation
        expanded = conversation.expanded
        focused = Int(conversation.focus ?? 0)
        // A request is about the page it was made of.
        documentRequest = nil
        // The rail starts where the pane lands, marked without asking the
        // page to go anywhere it is not already going.
        rail.setConversation(count: UInt32(conversation.rows.count))
        if let focus = conversation.focus {
            _ = rail.observed(index: focus)
        }
        marked = rail.marked().map(Int.init)
    }

    /// Show nothing.
    ///
    /// For landing on a message the store has not threaded. `messages.thread_id`
    /// is nullable — `ON DELETE SET NULL`, and the threading code sets it to
    /// NULL outright — so a row with no thread is reachable, and without this
    /// the pane went on drawing the *last* conversation underneath the new
    /// selection. The boundary guards the other half of the same thing and
    /// says why: *"a pane still drawing the previous conversation under a new
    /// selection is worse than an empty one, because it looks like an
    /// answer."*
    ///
    /// Everything `show` resets is reset here too, or the next conversation
    /// opens wearing the previous one's expansions.
    public func clear() {
        conversation = nil
        expanded = []
        focused = 0
        documentRequest = nil
        rail.setConversation(count: 0)
        marked = nil
    }

    /// Open or close the body of message `index`.
    public func toggle(_ index: Int) {
        guard expanded.indices.contains(index) else { return }
        expanded[index].toggle()
    }

    /// Open every message — `⌘⇧E`, the header's own control.
    public func expandAll() {
        expanded = expanded.map { _ in true }
        ask(.expandAll)
    }

    /// Move the keyboard to the next message of the conversation.
    ///
    /// Stops at the end rather than wrapping: a conversation has a last
    /// message, and arriving back at the top having pressed `J` once too
    /// often is a small lie about where you are.
    public func focusNext() {
        follow(rail.nextMessage())
    }

    /// Move the keyboard to the previous message.
    public func focusPrevious() {
        follow(rail.previousMessage())
    }

    /// A rail row was chosen: mark it, and take the pane there.
    public func choose(_ index: Int) {
        guard rows.indices.contains(index) else { return }
        follow(rail.activate(index: UInt32(index)))
    }

    /// The page reports which message fills the pane. The mark follows it;
    /// the pane is already there. Ignored while a chosen scroll is still
    /// under way -- the rail's rule.
    public func observed(message: Int64) {
        guard let index = rows.firstIndex(where: { $0.id == message }) else { return }
        if rail.observed(index: UInt32(index)).markMoved { adoptMark() }
    }

    /// The page finished a scroll the rail asked for; it may listen again.
    public func settled(_ settle: UInt64) {
        rail.settled(scroll: settle)
    }

    /// Take the rail's answer: move the mark, and the pane when it must go.
    private func follow(_ effect: RailEffectFfi) {
        guard effect.markMoved else { return }
        adoptMark()
        guard let message = focusedMessage else { return }
        ask(.scrollTo(message: message, settle: effect.scroll))
    }

    /// The marked message is the one the keyboard is on.
    private func adoptMark() {
        marked = rail.marked().map(Int.init)
        if let marked { focused = marked }
    }

    /// Fold or unfold the message the keyboard is on — `Space`.
    public func toggleFocused() {
        toggle(focused)
        if let message = focusedMessage { ask(.toggle(message: message)) }
    }
}
