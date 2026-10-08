import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// The header strip's words from the counts (screen 01): "312 · 41
/// unread", "Has action · 7", and on the right the filtered count only
/// while it says something (C10).
@Suite struct HeaderStripTests {
    static let keys: [String: String] = [
        "go_to_folders": "g o",
        "toggle_has_action": "!",
        "go_to_filtered": "g f",
        "go_to_digest_rules": "g d",
    ]

    static func strip(
        _ conversations: UInt32, unread: UInt32, hasAction: UInt32 = 7, filtered: UInt32 = 0,
        on: Bool = false
    ) -> HeaderStripWords {
        HeaderStripWords(
            counts: FocusCountsFfi(
                conversations: conversations, unread: unread, hasAction: hasAction,
                filteredToday: filtered),
            hasActionOn: on
        ) { keys[$0] }
    }

    @Test func theInboxSaysItsConversationsAndHowManyAreUnread() {
        let words = Self.strip(312, unread: 41)
        #expect(words.place == "Inbox")
        #expect(words.counts == "312 \u{b7} 41 unread")
        #expect(words.hasAction == "Has action \u{b7} 7")
    }

    @Test func nothingUnreadIsTheConversationsAlone() {
        #expect(Self.strip(312, unread: 0).counts == "312")
    }

    @Test func aLargeCountIsGrouped() {
        #expect(Self.strip(12_480, unread: 1_204).counts == "12,480 \u{b7} 1,204 unread")
    }

    @Test func theFilteredCountIsShownOnlyWhileItSaysSomething() {
        // C10: header counts only while in use.
        #expect(Self.strip(312, unread: 41, filtered: 0).filtered == nil)
        #expect(Self.strip(312, unread: 41, filtered: 186).filtered == "186 filtered today")
    }

    @Test func digestRulesAreNotDrawnUntilTheirCountCrosses() {
        // No FFI says how many digest rules there are yet, and a made-up
        // count would be a claim about the user's configuration.
        #expect(Self.strip(312, unread: 41).digestRules == nil)
    }

    @Test func everyCapIsTheUsersBinding() {
        let words = Self.strip(312, unread: 41, filtered: 3)
        #expect(words.placeCap == "g o")
        #expect(words.hasActionCap == "!")
        #expect(words.filteredCap == "g f")
        let rebound = HeaderStripWords(
            counts: FocusCountsFfi(conversations: 1, unread: 0, hasAction: 0, filteredToday: 0),
            hasActionOn: false
        ) { $0 == "toggle_has_action" ? "shift+1" : nil }
        #expect(rebound.hasActionCap == "⇧1")
        #expect(rebound.placeCap == nil)
    }

    @Test func beforeTheCountsArriveTheStripSaysNoNumber() {
        let words = HeaderStripWords(counts: nil, hasActionOn: false) { _ in nil }
        #expect(words.counts == nil)
        #expect(words.hasAction == "Has action")
    }
}

/// The toolbar's sync label (`postio_ui::focus_state::sync_label`'s words).
@Suite struct SyncLabelTests {
    static let zone = TimeZone(identifier: "Europe/Lisbon")!

    @Test func aFailureOutranksEverything() {
        let label = SyncLabel(offline: true, failing: true, syncing: (1, 2), lastSynced: 0, zone: Self.zone)
        #expect(label.text == "Sync failed")
    }

    @Test func offlineOutranksASyncThatCannotBeRunning() {
        let label = SyncLabel(offline: true, failing: false, syncing: (1, 2), lastSynced: 0, zone: Self.zone)
        #expect(label.text == "Offline")
    }

    @Test func aPassInFlightSaysHowFarItHasCome() {
        let label = SyncLabel(
            offline: false, failing: false, syncing: (1_200, 8_400), lastSynced: nil, zone: Self.zone)
        #expect(label.text == "Syncing 1,200 of 8,400")
    }

    @Test func aSettledStoreSaysWhenItLastSyncedAsAClockTime() {
        // 2026-09-26 15:09 UTC is 16:09 in Lisbon.
        let label = SyncLabel(
            offline: false, failing: false, syncing: nil, lastSynced: 1_790_435_340, zone: Self.zone)
        #expect(label.text == "Synced 16:09")
    }

    @Test func aStoreThatNeverSyncedSaysSo() {
        let label = SyncLabel(offline: false, failing: false, syncing: nil, lastSynced: nil, zone: Self.zone)
        #expect(label.text == "Not synced yet")
    }
}
