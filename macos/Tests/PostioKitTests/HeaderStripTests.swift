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

    static let strip = FocusStripFfi(
        counts: "312 \u{b7} 41 unread", hasAction: "Has action \u{b7} 7",
        filteredToday: "186 filtered today", digestRules: "4 digest rules")

    @Test func theWordsAreTheEnginesAsTheyCame() {
        // Composed in Rust by the functions GTK's strip uses; nothing here
        // words a count of its own (spec 009 FR-004).
        let words = HeaderStripWords(strip: Self.strip, hasActionOn: false) { Self.keys[$0] }
        #expect(words.place == "Inbox")
        #expect(words.counts == "312 \u{b7} 41 unread")
        #expect(words.hasAction == "Has action \u{b7} 7")
        #expect(words.filtered == "186 filtered today")
        #expect(words.digestRules == "4 digest rules")
    }

    @Test func theButtonNamesThePlaceTheListShows() {
        // `FocusPlace`: after `g a` or a pick in the popover, Inbox ▾ says
        // where the list is (screen 10).
        let words = HeaderStripWords(strip: Self.strip, place: "Receipts", hasActionOn: false) {
            Self.keys[$0]
        }
        #expect(words.place == "Receipts")
    }

    @Test func whatTheEngineLeavesOutIsNotDrawn() {
        // C10: the filtered and digest counts only while in use, decided in Rust.
        let quiet = FocusStripFfi(
            counts: "312", hasAction: "Has action \u{b7} 0", filteredToday: nil, digestRules: nil)
        let words = HeaderStripWords(strip: quiet, hasActionOn: false) { Self.keys[$0] }
        #expect(words.filtered == nil)
        #expect(words.digestRules == nil)
    }

    @Test func everyCapIsTheUsersBinding() {
        let words = HeaderStripWords(strip: Self.strip, hasActionOn: false) { Self.keys[$0] }
        #expect(words.placeCap == "g o")
        #expect(words.hasActionCap == "!")
        #expect(words.filteredCap == "g f")
        #expect(words.digestRulesCap == "g d")
        let rebound = HeaderStripWords(strip: Self.strip, hasActionOn: false) {
            $0 == "toggle_has_action" ? "shift+1" : nil
        }
        #expect(rebound.hasActionCap == "⇧1")
        #expect(rebound.placeCap == nil)
    }

    @Test func beforeTheWordsArriveTheStripSaysNoNumber() {
        let words = HeaderStripWords(strip: nil, hasActionOn: false) { _ in nil }
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
