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
        let table = NSTableView()
        table.headerView = nil
        table.style = .inset
        table.rowHeight = 62
        table.usesAutomaticRowHeights = false
        table.allowsMultipleSelection = true
        table.dataSource = controller
        table.delegate = controller

        let column = NSTableColumn(identifier: NSUserInterfaceItemIdentifier("message"))
        column.resizingMask = .autoresizingMask
        table.addTableColumn(column)

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        controller.tableView = table
        return scroll
    }

    public func updateNSView(_ scroll: NSScrollView, context: Context) {
        // Nothing to push: the table pulls. Reloads happen when a page lands,
        // through `MessageTableController.pageArrived`, and are scoped to the
        // rows that changed rather than the whole table.
    }
}
