import AppKit
import SwiftUI
import Testing

@testable import PostioAppKit
@testable import PostioKit

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
