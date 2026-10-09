import AppKit
import PostioFFI
import PostioKit
import QuickLookUI
import SwiftUI

/// The Files tab on the Mac (specs/010-focus-search T130; design §3.8,
/// screen 11): an `NSCollectionView` of `FileCardView`s, four across with
/// 12 between them and 20 at either side, in the results table's place.
///
/// The ring is the controller's (`FocusResultsCursor`), drawn by the card.
/// The arrows are the grid's, as any collection view's are -- ← and → a
/// card, ↑ and ↓ a row of four -- and say where the ring goes with
/// `focusSearchPoint`; the key monitor leaves them to it
/// (`KeyDisposition.belongsToGrid`). Every other key -- Space, ↩, ⌘↓, j/k
/// -- is a command the controller answers.
@MainActor
public final class FilesGrid: NSObject {
    /// Cards across (§3.8).
    public nonisolated static let columns = 4
    /// Between cards, across and down.
    public static let gap: CGFloat = 12
    /// Either side of the grid, and under its last row.
    public static let inset: CGFloat = 20

    public let model: ResultsModel
    public let collectionView: FilesCollectionView
    public let scrollView: NSScrollView

    /// The system's Quick Look over a file, which the grid lets take the
    /// panel's control when Space has asked for it.
    public var preview: FilePreview? {
        get { collectionView.preview }
        set { collectionView.preview = newValue }
    }

    static let itemIdentifier = NSUserInterfaceItemIdentifier("postio.files.card")

    public init(model: ResultsModel) {
        self.model = model
        collectionView = FilesCollectionView()
        scrollView = NSScrollView()
        super.init()
        configure()
    }

    private func configure() {
        let layout = Layout()
        layout.minimumInteritemSpacing = Self.gap
        layout.minimumLineSpacing = Self.gap
        layout.sectionInset = NSEdgeInsets(top: 0, left: Self.inset, bottom: Self.inset, right: Self.inset)
        collectionView.collectionViewLayout = layout
        collectionView.isSelectable = false
        collectionView.backgroundColors = [.textBackgroundColor]
        collectionView.dataSource = self
        collectionView.delegate = self
        collectionView.register(FileCardItem.self, forItemWithIdentifier: Self.itemIdentifier)
        collectionView.grid = self
        collectionView.setAccessibilityLabel(focusSearchWords().filesLabel)
        scrollView.documentView = collectionView
        scrollView.hasVerticalScroller = true
        scrollView.drawsBackground = true
        scrollView.backgroundColor = .textBackgroundColor
    }

    /// A card's size in a grid `width` wide: four across, the gaps and the
    /// insets taken out.
    public func itemSize(forWidth width: CGFloat) -> NSSize {
        let columns = CGFloat(Self.columns)
        let room = width - 2 * Self.inset - (columns - 1) * Self.gap
        return NSSize(width: max(120, room / columns), height: FileCardView.Metrics.height)
    }

    // MARK: what the engine says

    /// Draw what one of the model's changes said.
    public func apply(_ change: ResultsModel.Change) {
        guard model.isFiles else { return }
        switch change {
        case .open, .redraw, .close, .rows:
            collectionView.reloadData()
            scrollToCursor()
        case let .cursor(previous):
            var paths = Set<IndexPath>()
            for position in [previous.map(UInt64.init), model.cursor].compactMap({ $0 })
            where position < UInt64(model.cardCount) {
                paths.insert(IndexPath(item: Int(position), section: 0))
            }
            collectionView.reloadItems(at: paths)
            scrollToCursor()
        }
    }

    private func scrollToCursor() {
        guard let cursor = model.cursor, cursor < UInt64(model.cardCount) else { return }
        collectionView.scrollToItems(at: [IndexPath(item: Int(cursor), section: 0)], scrollPosition: .nearestHorizontalEdge)
    }

    /// Bring the keyboard to the grid: `FocusKeyboardHome` on the Files tab.
    public func takeKeyboard() {
        guard let window = collectionView.window else { return }
        window.makeFirstResponder(collectionView)
    }

    // MARK: the keyboard and the pointer

    /// An arrow on the grid.
    public enum Arrow: Equatable, Sendable {
        case left, right, up, down
    }

    /// Where `arrow` takes the ring from `cursor` among `count` cards,
    /// `columns` across; `nil` when it goes nowhere. Down from a row with
    /// a shorter row under it lands on that row's last card, as Finder's
    /// grid does.
    public nonisolated static func target(
        from cursor: UInt64?, arrow: Arrow, count: UInt64, columns: UInt64 = UInt64(FilesGrid.columns)
    ) -> UInt64? {
        guard count > 0 else { return nil }
        guard let cursor else { return 0 }
        switch arrow {
        case .left: return cursor > 0 ? cursor - 1 : nil
        case .right: return cursor + 1 < count ? cursor + 1 : nil
        case .up: return cursor >= columns ? cursor - columns : nil
        case .down:
            if cursor + columns < count { return cursor + columns }
            let last = count - 1
            return last / columns > cursor / columns ? last : nil
        }
    }

    /// Say where `arrow` takes the ring; whether it went anywhere.
    @discardableResult
    func move(_ arrow: Arrow) -> Bool {
        guard let target = Self.target(from: model.cursor, arrow: arrow, count: UInt64(model.cardCount)) else {
            return false
        }
        model.point(card: target)
        return true
    }
}

extension FilesGrid: NSCollectionViewDataSource {
    public func collectionView(_ collectionView: NSCollectionView, numberOfItemsInSection section: Int) -> Int {
        model.cardCount
    }

    public func collectionView(
        _ collectionView: NSCollectionView, itemForRepresentedObjectAt indexPath: IndexPath
    ) -> NSCollectionViewItem {
        let item = collectionView.makeItem(withIdentifier: Self.itemIdentifier, for: indexPath)
        let position = UInt64(indexPath.item)
        (item as? FileCardItem)?.show(
            FileCardView(card: model.card(at: position), focused: model.cursor == position))
        return item
    }
}

extension FilesGrid: NSCollectionViewDelegateFlowLayout {
    public func collectionView(
        _ collectionView: NSCollectionView, layout collectionViewLayout: NSCollectionViewLayout,
        sizeForItemAt indexPath: IndexPath
    ) -> NSSize {
        itemSize(forWidth: collectionView.bounds.width)
    }
}

extension FilesGrid {
    /// The flow, laid out again when the grid's width changes, so four
    /// cards stay four across.
    final class Layout: NSCollectionViewFlowLayout {
        override func shouldInvalidateLayout(forBoundsChange newBounds: NSRect) -> Bool {
            newBounds.width != collectionView?.bounds.width
        }
    }
}

/// One card's cell: the SwiftUI card, hosted.
final class FileCardItem: NSCollectionViewItem {
    private var host: NSHostingView<FileCardView>?

    override func loadView() {
        view = NSView()
    }

    func show(_ card: FileCardView) {
        if let host {
            host.rootView = card
            return
        }
        let made = NSHostingView(rootView: card)
        made.sizingOptions = []
        made.frame = view.bounds
        made.autoresizingMask = [.width, .height]
        view.addSubview(made)
        host = made
    }
}

/// The grid's view: its arrows, its clicks, and Quick Look's panel while
/// Space has a file in it.
public final class FilesCollectionView: NSCollectionView {
    weak var grid: FilesGrid?
    weak var preview: FilePreview?

    public override var acceptsFirstResponder: Bool { true }

    public override func keyDown(with event: NSEvent) {
        let arrow: FilesGrid.Arrow? = switch event.keyCode {
        case 123: .left
        case 124: .right
        case 125: .down
        case 126: .up
        default: nil
        }
        let chord = event.modifierFlags.intersection([.command, .option, .control, .shift])
        guard let arrow, chord.isEmpty, let grid else {
            super.keyDown(with: event)
            return
        }
        grid.move(arrow)
    }

    public override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        let point = convert(event.locationInWindow, from: nil)
        guard let grid, let path = indexPathForItem(at: point) else { return }
        let position = UInt64(path.item)
        if event.clickCount > 1 {
            grid.model.open(card: position)
        } else {
            grid.model.point(card: position)
        }
    }

    // The system's Quick Look asks the responder chain who drives it: the
    // grid, while a copy is up (FR-053).

    public override func acceptsPreviewPanelControl(_ panel: QLPreviewPanel!) -> Bool {
        preview?.url != nil
    }

    public override func beginPreviewPanelControl(_ panel: QLPreviewPanel!) {
        preview?.begin(panel)
    }

    public override func endPreviewPanelControl(_ panel: QLPreviewPanel!) {
        preview?.end(panel)
    }
}

/// The grid in a SwiftUI tree: the Files tab's body.
public struct FilesGridRepresentable: NSViewRepresentable {
    private let grid: FilesGrid

    public init(grid: FilesGrid) {
        self.grid = grid
    }

    public func makeNSView(context: Context) -> NSScrollView {
        grid.scrollView
    }

    public func updateNSView(_ scroll: NSScrollView, context: Context) {}
}
