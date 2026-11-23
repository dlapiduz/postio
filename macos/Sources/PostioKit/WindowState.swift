import CoreGraphics
import PostioFFI

/// Size, position and split widths are `NSWindow` autosave and `SceneStorage`. Postio owns two
/// decisions that fail quietly: which folder to reopen when the remembered one is gone, and where
/// to put a window saved on a vanished display.
public enum WindowState {
    /// A remembered mailbox can be renamed, unsubscribed or turned into a container, so the id is a
    /// hint: fall back to the inbox rather than show an empty list under a gone folder.
    public static func folderToOpen(remembered: Int64?, among folders: [MailboxFfi]) -> Int64? {
        // A `\Noselect` folder is a hierarchy node: opening it shows nothing, even if it was
        // remembered.
        let openable = folders.filter(\.selectable)
        if let remembered, openable.contains(where: { $0.id == remembered }) {
            return remembered
        }
        return openable.first { $0.role == .inbox }?.id ?? openable.first?.id
    }

    /// `NSWindow` autosave misses a window saved on a display no longer attached, which reopens at
    /// unreachable coordinates. Size is kept where it can be; one saved on a large display is
    /// shrunk to fit a laptop.
    public static func visibleFrame(for saved: CGRect, on screens: [CGRect]) -> CGRect {
        // Inventing a frame is worse than letting the window server place it.
        guard !screens.isEmpty else { return saved }

        // "Enough showing to grab": a sliver at the edge is not enough.
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

    /// Area, not distance, so a window straddling two displays lands on the one it was mostly on.
    private static func overlap(_ screen: CGRect, _ frame: CGRect) -> CGFloat {
        let shared = screen.intersection(frame)
        return shared.isNull ? 0 : shared.width * shared.height
    }
}
