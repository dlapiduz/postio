import PostioFFI

/// When the sidebar has to read its folders again.
///
/// # The bug this is
///
/// The badges were refreshed on `mailboxesChanged` and on nothing else. The
/// engine emits that only when the folder *set* moves — a folder created,
/// renamed or unsubscribed — which is folder discovery, not mail. So reading
/// a message did not decrement Inbox, archiving one moved nothing, and new
/// mail arriving left every count where it was. Switching folders happened to
/// re-emit the event, so everything jumped at once and the numbers looked
/// merely *slow* rather than wrong, which is why it survived.
///
/// # The rule
///
/// Counts move with **read state** and with mail **arriving or leaving**.
/// Which mailbox is irrelevant: the sidebar shows all of them, so a message
/// marked read in one folder can change a badge two accounts away if it was
/// in the unified scope.
///
/// `postio-gtk`'s `feed.rs` states the same rule in the same words, and the
/// two are supposed to stay the same — a sidebar that updates on Linux and
/// not on macOS is the drift ADR 0019 Q6 is about. It is not *shared* code,
/// because the two frontends receive different event types across different
/// seams; what is shared is the sentence, and this test file is what holds
/// this copy to it.
///
/// # Why this is a function and not four lines in `Engine.handle`
///
/// `Engine` is in the executable target and nothing can test it. Every
/// decision that ends up there is a decision with no test, which is how the
/// original rule came to be "one event" without anybody noticing.
public enum SidebarCounts {
    /// Whether `event` can have moved a folder's counts.
    ///
    /// A folder read is a store round trip per account, so the answer is `no`
    /// for everything that cannot have. `cursorMoved` matters most: it fires
    /// on every `j`.
    public static func movedBy(_ event: UiEvent) -> Bool {
        switch event {
        case .messagesChanged, .messagesRemoved, .newMail, .messageListChanged:
            // Read state, and mail arriving or leaving.
            return true
        case .mailboxesChanged:
            // The tree itself: created, renamed, unsubscribed. The rows
            // change, not only their numbers.
            return true
        case .conversationReady, .pageReady, .cursorMoved, .connectionChanged,
             .reindexProgress, .syncProgress, .other:
            return false
        }
    }
}
