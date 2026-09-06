import AppKit
import SwiftUI

/// The message list, as SwiftUI can hold it.
///
/// An `NSViewRepresentable` around `NSTableView` rather than a SwiftUI `List`.
/// `List` wraps `NSTableView` anyway and hides the two things this needs: real
/// cell reuse, and explicit scroll-anchor control when new mail arrives at the
/// top — the GTK side deliberately moves the anchor down with its row so the
/// cursor stays on the message being read, and `List` offers no equivalent.
public struct MessageListView: NSViewRepresentable {
    private let controller: MessageTableController

    public init(controller: MessageTableController) {
        self.controller = controller
    }

    public func makeNSView(context: Context) -> NSScrollView {
        Self.makeTable(controller: controller)
    }

    /// Build the scrolling table this view wraps.
    ///
    /// Separated from `makeNSView` for the reason
    /// `MessageTableController.cell(reusing:)` is separated from the delegate
    /// method: a SwiftUI `Context` cannot be constructed in a test, and
    /// everything decided here -- the row height the density asks for above
    /// all -- is ours to get wrong.
    static func makeTable(controller: MessageTableController) -> NSScrollView {
        let table = NSTableView()
        table.headerView = nil
        table.style = .inset
        // From the cell, not a literal. A row shorter than its contents clips
        // the sender on every row it draws, which is what `62` did.
        table.rowHeight = controller.rowHeight
        table.usesAutomaticRowHeights = false
        // The table's own selection is the **cursor**, and only ever one row.
        // The multi-message selection is Postio's, lives behind the boundary,
        // and is drawn per row -- `PRODUCT.md` §9. Left at `true`,
        // `NSTableView` would conflate them and shift-click would destroy
        // what the user had built up.
        table.allowsMultipleSelection = false
        table.dataSource = controller
        table.delegate = controller

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)

        // Named, so the keyboard can be moved here on purpose and a screen
        // reader says which of the three panes it is in.
        table.setAccessibilityLabel(Pane.list.label)

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        controller.tableView = table
        // The mouse's path to the three verbs. Present whatever
        // `show_hover_actions` says: off means the mouse reaches them another
        // way, never through nothing.
        let menu = NSMenu()
        menu.delegate = controller
        table.menu = menu
        return scroll
    }

    public func updateNSView(_ scroll: NSScrollView, context: Context) {
        // Nothing to push: the table pulls. Reloads happen when a page lands,
        // through `MessageTableController.pageArrived`, and are scoped to the
        // rows that changed rather than the whole table.
    }
}
