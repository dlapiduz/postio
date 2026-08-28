import CoreGraphics
import PostioFFI

/// What the window remembers between launches.
///
/// Size, position and split widths are `NSWindow` frame autosave and
/// `SceneStorage`, which are Apple's to get right. What is Postio's is the two
/// decisions underneath, and both of them fail quietly: which folder to reopen
/// when the remembered one is gone, and where to put a window that was saved
/// on a display nobody has any more.
///
/// `postio-gtk/src/state.rs` is the Linux half of this. It is not shared,
/// deliberately — a saved GTK pane position is not a saved `NSSplitView`
/// width, and the *storage* is where the two platforms genuinely differ. What
/// is shared is the idea that the last folder is application state rather than
/// window state.
public enum WindowState {
    /// Which folder to open, given what was remembered and what still exists.
    ///
    /// A remembered mailbox can be renamed, unsubscribed, or turned into a
    /// container between launches, so the id is a hint and never a promise.
    /// Falling back to the inbox is right; an empty list under the name of a
    /// folder that is gone is the failure this guards.
    public static func folderToOpen(remembered: Int64?, among folders: [MailboxFfi]) -> Int64? {
        // A `\Noselect` folder is a node in the hierarchy rather than a
        // mailbox: opening it shows nothing, so it is not a candidate even
        // when it is exactly what was remembered.
        let openable = folders.filter(\.selectable)
        if let remembered, openable.contains(where: { $0.id == remembered }) {
            return remembered
        }
        // The inbox, then whatever is first. An account with no inbox is
        // unusual and not an error, and opening its first folder beats opening
        // none.
        return openable.first { $0.role == .inbox }?.id ?? openable.first?.id
    }

    /// A saved frame, moved onto a screen if it is not on one.
    ///
    /// `NSWindow`'s autosave handles the ordinary cases and not this one: a
    /// window saved on a second display that is no longer attached reopens at
    /// coordinates nothing can reach, and the application looks like it failed
    /// to launch.
    ///
    /// Size is preserved wherever it can be. A window saved on a large display
    /// and reopened on a laptop cannot be moved into view — only resized — so
    /// that case shrinks it to fit rather than leaving it unreachable.
    public static func visibleFrame(for saved: CGRect, on screens: [CGRect]) -> CGRect {
        // Nothing better to say, and inventing a frame is worse than letting
        // the window server place it.
        guard !screens.isEmpty else { return saved }

        // "Enough of it showing to grab." A sliver at the edge is not enough,
        // and it is what dragging a window nearly off the right edge leaves.
        if screens.contains(where: { $0.contains(saved) }) { return saved }

        let target = screens.max(by: { overlap($0, saved) < overlap($1, saved) }) ?? screens[0]
        let size = CGSize(
            width: min(saved.width, target.width),
            height: min(saved.height, target.height)
        )
        return CGRect(
            x: min(max(saved.minX, target.minX), target.maxX - size.width),
            y: min(max(saved.minY, target.minY), target.maxY - size.height),
            width: size.width,
            height: size.height
        )
    }

    /// How much of `frame` falls on `screen`, for picking the best one.
    ///
    /// Area rather than distance, so a window straddling two displays lands on
    /// the one it was mostly on rather than the one whose centre is nearest.
    private static func overlap(_ screen: CGRect, _ frame: CGRect) -> CGFloat {
        let shared = screen.intersection(frame)
        return shared.isNull ? 0 : shared.width * shared.height
    }
}
