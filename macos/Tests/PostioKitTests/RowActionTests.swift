import AppKit
import PostioFFI
import Testing

@testable import PostioKit

/// The mouse's path to the three verbs triage is made of.
///
/// The list had none: no hover actions, no context menu, no way to archive a
/// message without the keyboard. `show_hover_actions` had nothing to turn off
/// because there was nothing there.
@MainActor
@Suite struct RowActionTests {
    private func controller() -> MessageTableController {
        let controller = MessageTableController(source: StubRowSource(rowCount: 10))
        let scroll = MessageListView.makeTable(controller: controller)
        controller.tableView = scroll.documentView as? NSTableView
        controller.tableView?.reloadData()
        return controller
    }

    @Test func theContextMenuCarriesTheSameThreeVerbsTheKeyboardDoes() {
        // They are registry command ids, not local implementations: a mouse
        // path that did its own thing would be a fourth way to archive that
        // undo did not know about.
        let menu = MessageTableController.rowMenu(for: 3, flagged: false)
        #expect(menu.items.map(\.title) == ["Archive", "Flag", "Delete"])
        #expect(menu.items.compactMap { $0.representedObject as? String }
            == ["archive", "flag", "delete"])
    }

    @Test func theContextMenuIsThereEvenWithHoverActionsTurnedOff() {
        // GTK's rule, and the reason this is not simply "hover actions": off
        // means the mouse reaches the same verbs another way, never through
        // nothing.
        let controller = controller()
        controller.ui.showHoverActions = false
        #expect(controller.menu(forRow: 2)?.items.isEmpty == false)
    }

    @Test func choosingOneRunsItOnTheRowItWasOpenedOn() {
        // The verb acts on the message under the pointer, not on wherever the
        // cursor happens to be — that is the whole difference between a
        // context menu and a keystroke.
        let controller = controller()
        var ran: [(String, Int)] = []
        controller.onRowAction = { command, row in ran.append((command, row)) }

        let menu = controller.menu(forRow: 4)
        let flag = try? #require(menu?.items.first { $0.title == "Flag" })
        controller.runRowAction(flag!)

        #expect(ran.count == 1)
        #expect(ran.first?.0 == "flag")
        #expect(ran.first?.1 == 4)
    }

    @Test func theHoverButtonsAppearOnlyWhenTheSettingIsOn() {
        let on = MessageRowCell()
        on.ui.showHoverActions = true
        on.hovered = true
        #expect(!on.actionsAreHiddenForTesting)

        let off = MessageRowCell()
        off.ui.showHoverActions = false
        off.hovered = true
        #expect(off.actionsAreHiddenForTesting)
    }

    @Test func theHoverButtonsGoAwayWhenThePointerLeaves() {
        let cell = MessageRowCell()
        cell.ui.showHoverActions = true
        cell.hovered = true
        cell.hovered = false
        #expect(cell.actionsAreHiddenForTesting)
    }

    @Test func aFlaggedRowOffersToUnflagIt() {
        // The state, not the verb: the glyph has to say which way it would go.
        #expect(
            MessageTableController.rowMenu(for: 0, flagged: true).items
                .first { ($0.representedObject as? String) == "flag" }?.title == "Unflag")
    }
}

/// The verb reaching the boundary from the row it was asked on.
@MainActor
@Suite struct RowActionWiringTests {
    @Test func aHoverButtonReportsTheRowItWasPressedOn() {
        // The cell knows the verb and only the controller knows the row, so
        // this is the join between them — and the join is where a mouse path
        // ends up acting on the wrong message.
        let controller = MessageTableController(source: StubRowSource(rowCount: 10))
        let scroll = MessageListView.makeTable(controller: controller)
        let table = scroll.documentView as! NSTableView
        controller.tableView = table
        table.reloadData()

        var ran: [(String, Int)] = []
        controller.onRowAction = { command, row in ran.append((command, row)) }

        let cell = controller.tableView(table, viewFor: table.tableColumns.first, row: 5)
            as? MessageRowCell
        cell?.onAction?("archive")

        #expect(ran.first?.0 == "archive")
        #expect(ran.first?.1 == 5)
    }

    @Test func theTableOffersAContextMenuAtAll() {
        // The regression this guards: the list had no `menu` set, so a
        // right-click did nothing and the mouse could not archive anything.
        let controller = MessageTableController(source: StubRowSource(rowCount: 3))
        let scroll = MessageListView.makeTable(controller: controller)
        let table = scroll.documentView as! NSTableView
        #expect(table.menu != nil)
        #expect(table.menu?.delegate === controller)
    }
}
