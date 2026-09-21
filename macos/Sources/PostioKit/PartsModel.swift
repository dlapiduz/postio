import Observation
import PostioFFI

/// The parts panel's own state.
///
/// # What this is for
///
/// **An attachment that arrived on a Mac could not be saved.** There was no
/// parts surface and no other route to a message's MIME tree, so a message
/// with a PDF on it showed its body and nothing else. Eight registry commands
/// drove GTK's panel; every one of them reached nothing here.
///
/// # What is local, and what is not
///
/// The **cursor** is local, for the same reason the list's is: it is a
/// position in a view, and moving it changes nothing anybody else can see.
/// Everything else comes from the boundary — the tree, the box-drawing
/// prefixes, what a row is called, what a row *says* to a screen reader, and
/// above all the filename a part may be written under. Two frontends
/// disagreeing about which bytes are "the attachment", or about how a
/// sender's `filename=` is made safe, is not a cosmetic difference.
///
/// A `filename` from the wire is attacker-controlled text and never reaches
/// the filesystem: `PartFfi.saveName` is the one that may, and it is the
/// boundary's answer.
@MainActor
@Observable
public final class PartsModel {
    /// The message's parts, as the boundary read them.
    public private(set) var parts: [PartFfi] = []
    /// What the panel says it is showing: `3 parts · 4.2 MB`.
    public private(set) var summary = ""
    /// Which row the keyboard is on.
    public private(set) var cursor: UInt32 = 0

    public init() {}

    /// Show `read`, forgetting the last message's position.
    ///
    /// The cursor is a position in *this* message's tree; carrying it across
    /// would land on whatever part of the next message shared the index.
    public func show(_ read: MessagePartsFfi) {
        parts = read.parts
        summary = read.summary
        // The boundary's answer, not zero: the message itself is row zero and
        // is never the interesting row, so where a panel opens is a decision
        // rather than a default.
        cursor = min(read.cursor, UInt32(max(parts.count, 1) - 1))
    }

    /// Forget everything — the reader is showing something else now.
    public func clear() {
        parts = []
        summary = ""
        cursor = 0
    }

    /// The row the keyboard is on.
    public var focused: PartFfi? {
        parts.indices.contains(Int(cursor)) ? parts[Int(cursor)] : nil
    }

    /// Move the keyboard one row.
    ///
    /// Clamped rather than wrapping, and the arithmetic is the boundary's so
    /// both panels walk alike.
    public func step(forward: Bool) {
        guard !parts.isEmpty else { return }
        cursor = partCursorAfter(current: cursor, forward: forward, count: UInt32(parts.count))
    }

    /// Whether the focused row is something there are bytes to save.
    ///
    /// A container holds other parts and has nothing of its own, so Save on
    /// it is a verb that cannot run. A part that has **not arrived yet** is a
    /// different matter and stays offered: asking for it is what fetches it,
    /// and "not downloaded" is the ordinary state of an attachment on a
    /// message that has only been described.
    public var canSaveFocused: Bool {
        guard let focused else { return false }
        return !focused.isContainer
    }
}
