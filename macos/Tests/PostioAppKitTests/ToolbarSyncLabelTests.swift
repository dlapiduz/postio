import AppKit
import SwiftUI
import Testing

@testable import PostioAppKit
@testable import PostioKit

/// The toolbar's sync label sits in the glass capsule it shares with the
/// search field (macOS 26), whose rounded end cut into its symbol while
/// the label was only as wide as its symbol and words.
@MainActor
@Suite struct ToolbarSyncLabelTests {
    @Test func theLabelKeepsRoomBeforeItsSymbolAndAfterItsWords() {
        let label = SyncLabel(text: "Syncing 19 of 39", mark: .syncing)
        let drawn = NSHostingView(rootView: ToolbarSyncLabel(label: label)).fittingSize.width
        let bare = NSHostingView(rootView: ToolbarSyncLabel.Words(label: label)).fittingSize.width
        #expect(drawn - bare >= ToolbarSyncLabel.leading + ToolbarSyncLabel.trailing)
        #expect(ToolbarSyncLabel.leading >= 10, "clear of the capsule's rounded end")
    }
}
