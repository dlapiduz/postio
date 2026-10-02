import PostioFFI

/// Whether an engine event means the conversation page on screen has to be
/// composed again.
///
/// Every sync pass of every folder says "messages changed", and the engine
/// used to bump the page's revision for all of them: each bump composed the
/// open conversation again -- a body load per message -- and cancelled the
/// compose a move between messages was waiting on. Now only an event that
/// touches a message on the page does. A list change names no messages and
/// may be a reply to this conversation arriving, so it still asks; the page
/// is loaded only if it came out different.
public enum PageRefresh {
    public static func needed(by event: UiEvent, showing: Set<Int64>) -> Bool {
        guard !showing.isEmpty else { return false }
        switch event {
        case let .messagesChanged(_, messages), let .messagesRemoved(_, _, messages):
            return messages.contains(where: showing.contains)
        case .messageListChanged:
            return true
        default:
            return false
        }
    }
}
