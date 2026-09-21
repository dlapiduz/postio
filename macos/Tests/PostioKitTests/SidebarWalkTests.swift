import PostioFFI
import Testing

@testable import PostioKit

/// Walking the folder tree from the keyboard.
///
/// `g f` moved the keyboard into the sidebar and then `j`, `k`, `space` and
/// the four `g` destinations all did nothing — they reached no handler at
/// all, and because the key monitor claimed them they did not reach the list
/// underneath either. Every way of changing folder without the mouse was
/// gone, and the `g` destinations are the keys somebody learns first.
///
/// The rules are GTK's `Sidebar::step`, down to the reasons:
/// **one list across every section**, because that is what it looks like;
/// **stops at the ends rather than wrapping**, because "wrapping a short list
/// is how you end up in Trash when you meant to stop at Inbox"; and from a
/// standing start `j` takes the top and `k` the bottom, so both keys reach a
/// row from nowhere.
@Suite struct SidebarWalkTests {
    private func folder(
        _ id: Int64,
        _ name: String,
        role: MailboxRoleFfi = .regular,
        parent: Int64? = nil,
        account: Int64 = 1,
        special: Bool = false,
        selectable: Bool = true
    ) -> MailboxFfi {
        MailboxFfi(
            id: id, account: account, parent: parent, name: name, role: role,
            unread: 0, total: 0, selectable: selectable, lastSyncedAt: nil,
            special: special, flagged: 0, snoozed: 0
        )
    }

    /// Inbox and Archive above; `Projects` with `2026` under it below.
    private func tree() -> (special: [MailboxFfi], roots: [MailboxFfi], all: [MailboxFfi]) {
        let special = [
            folder(1, "Inbox", role: .inbox, special: true),
            folder(2, "Archive", role: .archive, special: true),
        ]
        let projects = folder(10, "Projects")
        let year = folder(11, "2026", parent: 10)
        let quarter = folder(12, "Q1", parent: 11)
        let lists = folder(20, "Lists")
        return (special, [projects, lists], special + [projects, year, quarter, lists])
    }

    private func children(_ all: [MailboxFfi]) -> (Int64) -> [MailboxFfi] {
        { parent in all.filter { !$0.special && $0.parent == parent } }
    }

    @Test func theOrderIsWhatTheEyeSeesAcrossEverySection() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: []
        )
        #expect(
            order.map(\.name) == ["Inbox", "Archive", "Projects", "2026", "Q1", "Lists"],
            "the walk does not follow the drawn order: \(order.map(\.name))"
        )
    }

    @Test func aCollapsedFolderHidesItsDescendantsFromTheWalk() {
        // Stepping onto a row nobody can see is a cursor that has vanished.
        let (special, roots, all) = tree()
        let projects = roots[0].rowId
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: [projects]
        )
        #expect(order.map(\.name) == ["Inbox", "Archive", "Projects", "Lists"])
    }

    @Test func steppingStopsAtBothEndsRatherThanWrapping() {
        // "Wrapping a short list is how you end up in Trash when you meant to
        // stop at Inbox."
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.step(from: order.first?.rowId, in: order, by: -1)?.name == "Inbox")
        #expect(SidebarWalk.step(from: order.last?.rowId, in: order, by: 1)?.name == "Lists")
    }

    @Test func fromAStandingStartBothKeysReachARow() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.step(from: nil, in: order, by: 1)?.name == "Inbox")
        #expect(SidebarWalk.step(from: nil, in: order, by: -1)?.name == "Lists")
    }

    @Test func steppingMovesOneRowInTheDrawnOrder() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: []
        )
        let archive = order[1].rowId
        #expect(SidebarWalk.step(from: archive, in: order, by: 1)?.name == "Projects")
        #expect(SidebarWalk.step(from: archive, in: order, by: -1)?.name == "Inbox")
    }

    @Test func anEmptySidebarHasNowhereToStep() {
        #expect(SidebarWalk.step(from: nil, in: [], by: 1) == nil)
    }

    @Test func theGoToDestinationsFindTheirRoleWhereverItIs() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.destination(.inbox, among: order)?.name == "Inbox")
        #expect(SidebarWalk.destination(.archive, among: order)?.name == "Archive")
    }

    @Test func aRoleThisAccountHasNoFolderForIsAnswerNotSilence() {
        // GTK announces "This account has no drafts folder" rather than
        // swallowing the key. `nil` here is what the caller says that about —
        // the important thing is that it is distinguishable from "done".
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.destination(.drafts, among: order) == nil)
    }

    @Test func aContainerCanBeSteppedOntoAndNotOpened() {
        // A `\Noselect` container keeps its row so the hierarchy under it can
        // be reached. The keyboard has to be able to land on it — otherwise
        // there is no way to expand it — and landing must not open it as a
        // mailbox, because there is none behind it.
        let container = folder(30, "Archives", selectable: false)
        let child = folder(31, "2024", parent: 30)
        let order = SidebarWalk.visible(
            special: [], roots: [container], children: { $0 == 30 ? [child] : [] }, collapsed: []
        )
        #expect(order.map(\.name) == ["Archives", "2024"])
        #expect(SidebarWalk.opens(order[0]) == false, "a container has no mailbox to open")
        #expect(SidebarWalk.opens(order[1]))
    }
}
