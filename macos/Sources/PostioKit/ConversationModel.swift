import Foundation
import PostioFFI

/// The conversation the reading pane is showing, and what a reader has done
/// to it since it opened (#1263, ADR 0015 Q4).
///
/// **It decides as little as possible.** Where a conversation opens, which
/// messages are expanded when it does, and which runs of collapsed messages
/// fold into a divider are all `postio_ui::conversation`'s answers, arriving
/// through the boundary already made — because those are the decisions that
/// cost something (one web view per expanded message) and the ones two
/// frontends must not answer differently.
///
/// What is genuinely local is what a person then does: expand this one,
/// collapse that one, show me the five you folded away. That is this type.
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

    /// Runs the reader has asked to see, by the index they start at.
    ///
    /// Keyed on the start index rather than on identity because a run *is* a
    /// position — expanding something inside a conversation reshapes the
    /// runs, and a reveal that survived that would be about a run nobody is
    /// looking at any more.
    private var revealed: Set<Int> = []

    /// Which messages have had their `Cc` list opened, by index (#1259).
    ///
    /// Per message, never per pane: a conversation can hold eight messages
    /// addressed to eight different lists, and one disclosure standing for
    /// all of them would be about none of them — the same reason the blocked
    /// images notice is per message.
    private var ccRevealed: Set<Int> = []

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
        revealed = []
        // The disclosures are about *these* messages. Carrying them across
        // would open a recipient list somebody never asked to see, on
        // somebody else's mail.
        ccRevealed = []
        focused = Int(conversation.focus ?? 0)
    }

    /// Open or close the body of message `index`.
    public func toggle(_ index: Int) {
        guard expanded.indices.contains(index) else { return }
        expanded[index].toggle()
    }

    /// Open every message — `⌘⇧E`, the header's own control.
    public func expandAll() {
        expanded = expanded.map { _ in true }
    }

    /// Move the keyboard to the next message of the conversation.
    ///
    /// Stops at the end rather than wrapping: a conversation has a last
    /// message, and arriving back at the top having pressed `J` once too
    /// often is a small lie about where you are.
    public func focusNext() {
        guard !rows.isEmpty else { return }
        focused = min(focused + 1, rows.count - 1)
    }

    /// Move the keyboard to the previous message.
    public func focusPrevious() {
        guard !rows.isEmpty else { return }
        focused = max(focused - 1, 0)
    }

    /// Fold or unfold the message the keyboard is on — `Space`.
    public func toggleFocused() {
        toggle(focused)
    }

    /// The runs of collapsed messages long enough to hide behind a divider,
    /// minus the ones already revealed.
    public var runs: [RunFfi] {
        guard !expanded.isEmpty else { return [] }
        return PostioSession.runs(rows: rows, expanded: expanded)
            .filter { !revealed.contains(Int($0.start)) }
    }

    /// Show the messages a divider is standing in for.
    ///
    /// They arrive as the one-line headers they already were: revealing is
    /// about the divider, not about the bodies, so five hidden messages cost
    /// five lines rather than five web views.
    /// Whether message `index` is showing its `Cc` addresses.
    public func isCcRevealed(_ index: Int) -> Bool { ccRevealed.contains(index) }

    /// Open or close message `index`'s `Cc` list.
    public func toggleCc(_ index: Int) {
        if ccRevealed.contains(index) {
            ccRevealed.remove(index)
        } else {
            ccRevealed.insert(index)
        }
    }

    public func reveal(_ run: RunFfi) {
        revealed.insert(Int(run.start))
    }

    /// Whether message `index` is drawn at all — a message inside a folded
    /// run is not.
    public func isVisible(_ index: Int) -> Bool {
        !runs.contains { run in
            index >= Int(run.start) && index < Int(run.start) + Int(run.count)
        }
    }
}
