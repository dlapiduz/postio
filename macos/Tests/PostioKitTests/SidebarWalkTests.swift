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
            special: special, saved: [], roots: roots, children: children(all), collapsed: []
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
            special: special, saved: [], roots: roots, children: children(all), collapsed: [projects]
        )
        #expect(order.map(\.name) == ["Inbox", "Archive", "Projects", "Lists"])
    }

    @Test func steppingStopsAtBothEndsRatherThanWrapping() {
        // "Wrapping a short list is how you end up in Trash when you meant to
        // stop at Inbox."
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, saved: [], roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.step(from: order.first?.cursor, in: order, by: -1)?.name == "Inbox")
        #expect(SidebarWalk.step(from: order.last?.cursor, in: order, by: 1)?.name == "Lists")
    }

    @Test func fromAStandingStartBothKeysReachARow() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, saved: [], roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.step(from: nil, in: order, by: 1)?.name == "Inbox")
        #expect(SidebarWalk.step(from: nil, in: order, by: -1)?.name == "Lists")
    }

    @Test func steppingMovesOneRowInTheDrawnOrder() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, saved: [], roots: roots, children: children(all), collapsed: []
        )
        let archive = order[1].cursor
        #expect(SidebarWalk.step(from: archive, in: order, by: 1)?.name == "Projects")
        #expect(SidebarWalk.step(from: archive, in: order, by: -1)?.name == "Inbox")
    }

    @Test func anEmptySidebarHasNowhereToStep() {
        #expect(SidebarWalk.step(from: nil, in: [], by: 1) == nil)
    }

    @Test func theGoToDestinationsFindTheirRoleWhereverItIs() {
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, saved: [], roots: roots, children: children(all), collapsed: []
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
            special: special, saved: [], roots: roots, children: children(all), collapsed: []
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
            special: [], saved: [], roots: [container], children: { $0 == 30 ? [child] : [] }, collapsed: []
        )
        #expect(order.map(\.name) == ["Archives", "2024"])
        #expect(SidebarWalk.opens(order[0].folder!) == false, "a container has no mailbox to open")
        #expect(SidebarWalk.opens(order[1].folder!))
    }

    // MARK: Saved searches are rows too

    private func saved(_ key: String, _ name: String) -> SavedSearchFfi {
        SavedSearchFfi(key: key, name: name, query: "from:\(key)@example.com")
    }

    @Test func savedSearchesSitBetweenTheFavouritesAndTheFolders() {
        // Where the sidebar draws them: Favorites, then "Saved searches",
        // then "On My Mac". A walk that skipped them left `r`, `d`, ⇧↑ and
        // ⇧↓ acting on a cursor only the mouse could put anywhere (#1573).
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special,
            saved: [saved("ada", "From Ada"), saved("big", "Large")],
            roots: roots, children: children(all), collapsed: []
        )
        #expect(
            order.map(\.name)
                == ["Inbox", "Archive", "From Ada", "Large", "Projects", "2026", "Q1", "Lists"],
            "the walk does not follow the drawn order: \(order.map(\.name))"
        )
    }

    @Test func theWalkCrossesIntoAndOutOfTheSavedSearches() {
        // One list across every section: `j` from the last favourite lands on
        // the first saved search, and `j` from the last saved search lands on
        // the first folder, with no gesture in between.
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special,
            saved: [saved("ada", "From Ada"), saved("big", "Large")],
            roots: roots, children: children(all), collapsed: []
        )
        let archive = SidebarCursor.folder(special[1].rowId)
        #expect(SidebarWalk.step(from: archive, in: order, by: 1) == .savedSearch(saved("ada", "From Ada")))
        let large = SidebarCursor.savedSearch("big")
        #expect(SidebarWalk.step(from: large, in: order, by: 1)?.name == "Projects")
        #expect(SidebarWalk.step(from: large, in: order, by: -1)?.name == "From Ada")
    }

    @Test func aSavedSearchThatWentAwayIsAStandingStart() {
        // `config.toml` is hand-edited and watched, so the row under the
        // cursor can vanish between two key presses. Guessing a neighbour
        // would put the keyboard somewhere nobody sent it; the standing-start
        // rule already says where `j` and `k` go from nowhere.
        let (special, roots, all) = tree()
        let order = SidebarWalk.visible(
            special: special, saved: [saved("ada", "From Ada")],
            roots: roots, children: children(all), collapsed: []
        )
        #expect(SidebarWalk.step(from: .savedSearch("gone"), in: order, by: 1)?.name == "Inbox")
    }

    @Test func aGoToDestinationIsNeverASavedSearch() {
        // `g d` means the Drafts folder. A saved search called "Drafts" is a
        // query somebody wrote down, and landing on it would run a search
        // where the key promised a folder.
        let drafts = SavedSearchFfi(key: "drafts", name: "Drafts", query: "in:drafts")
        let order = SidebarWalk.visible(
            special: [folder(1, "Inbox", role: .inbox, special: true)],
            saved: [drafts], roots: [], children: { _ in [] }, collapsed: []
        )
        #expect(SidebarWalk.destination(.drafts, among: order) == nil)
    }

    @Test func theCursorIsTheSavedSearchWhileOneIsHeld() {
        // Two halves, one answer: the saved searches keep their own cursor
        // (it follows a row through a reorder) and the folder half stays put
        // underneath, so leaving the search puts the highlight back on the
        // folder that was open rather than on nothing.
        let inbox = folder(1, "Inbox", role: .inbox, special: true).rowId
        #expect(SidebarWalk.cursor(folder: inbox, savedSearch: "ada") == .savedSearch("ada"))
        #expect(SidebarWalk.cursor(folder: inbox, savedSearch: nil) == .folder(inbox))
        #expect(SidebarWalk.cursor(folder: nil, savedSearch: nil) == nil)
        #expect(SidebarWalk.highlightedFolder(folder: inbox, savedSearch: "ada") == nil,
                "a folder drawn selected beside a running saved search claims two rows at once")
        #expect(SidebarWalk.highlightedFolder(folder: inbox, savedSearch: nil) == inbox)
    }
}
