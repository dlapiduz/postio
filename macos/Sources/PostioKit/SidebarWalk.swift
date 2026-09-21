import PostioFFI

/// Walking the folder tree from the keyboard.
///
/// # The bug this is
///
/// `g f` moved the keyboard into the sidebar and then nothing walked. `j`,
/// `k`, `space` and the four `g` destinations reached no handler at all —
/// and because the key monitor claimed them, they did not reach the list
/// underneath either. Every way of changing folder without the mouse was
/// gone, and the `g` destinations are the keys somebody learns first in this
/// application.
///
/// # The rules are GTK's, and so are the reasons
///
/// `postio_gtk::sidebar::Sidebar::step` has had them since it had a tree:
///
/// - **One list across every section.** The sidebar looks like one column, so
///   `j` crosses from the last favourite into the first account folder
///   without a gesture in between.
/// - **Stops at the ends rather than wrapping.** *"Wrapping a short list is
///   how you end up in Trash when you meant to stop at Inbox."*
/// - **From a standing start, `j` takes the top and `k` the bottom**, so both
///   keys reach a row from nowhere.
/// - **A row you cannot see is not in the walk.** Stepping into a collapsed
///   group would leave the cursor somewhere nothing is drawn.
/// - **A `\Noselect` container can be landed on and not opened.** It keeps
///   its row so the hierarchy under it is reachable, and there is no mailbox
///   behind it to show.
///
/// It is here rather than in `Engine` because `Engine` is in the executable
/// target and nothing can test it — which is how the last sidebar rule came
/// to be written without one.
public enum SidebarWalk {
    /// Every row the sidebar is drawing, in the order it draws them.
    ///
    /// `collapsed` is the disclosure state, which the application has to hold
    /// rather than leave inside SwiftUI: a walk that cannot see it steps onto
    /// rows nobody can see, and `toggle_folder` has nothing to toggle.
    public static func visible(
        special: [MailboxFfi],
        roots: [MailboxFfi],
        children: (Int64) -> [MailboxFfi],
        collapsed: Set<SidebarRowId>
    ) -> [MailboxFfi] {
        // The favourites are drawn flat — one row per role, their tree is
        // under the accounts — so they contribute themselves and nothing else.
        var order = special
        for root in roots {
            append(root, children: children, collapsed: collapsed, to: &order)
        }
        return order
    }

    private static func append(
        _ folder: MailboxFfi,
        children: (Int64) -> [MailboxFfi],
        collapsed: Set<SidebarRowId>,
        to order: inout [MailboxFfi]
    ) {
        order.append(folder)
        guard !collapsed.contains(folder.rowId) else { return }
        for child in children(folder.id) {
            append(child, children: children, collapsed: collapsed, to: &order)
        }
    }

    /// The row `delta` away from `current` in `order`.
    ///
    /// `nil` only when there is nothing to step to at all.
    public static func step(
        from current: SidebarRowId?,
        in order: [MailboxFfi],
        by delta: Int
    ) -> MailboxFfi? {
        guard !order.isEmpty else { return nil }
        guard let current, let at = order.firstIndex(where: { $0.rowId == current }) else {
            // Nothing selected: `j` starts at the top and `k` at the bottom,
            // so both keys reach a row from a standing start.
            return delta > 0 ? order.first : order.last
        }
        // Clamped, not wrapped.
        let next = min(max(at + delta, 0), order.count - 1)
        return order[next]
    }

    /// The row a `go_to_*` destination means, or `nil` if this account has no
    /// folder for that role.
    ///
    /// `nil` is an answer the caller has to say out loud — GTK announces
    /// *"This account has no drafts folder"* rather than swallowing the key,
    /// because a key that silently does nothing is indistinguishable from one
    /// that is broken.
    public static func destination(
        _ role: MailboxRoleFfi,
        among order: [MailboxFfi]
    ) -> MailboxFfi? {
        order.first { $0.role == role }
    }

    /// Whether landing on `folder` should also open it.
    ///
    /// A `\Noselect` container has no mailbox behind it. The keyboard still
    /// has to be able to land there — otherwise there is no way to expand it
    /// — so stepping and opening are two questions, and this is the second.
    public static func opens(_ folder: MailboxFfi) -> Bool {
        folder.selectable
    }
}
