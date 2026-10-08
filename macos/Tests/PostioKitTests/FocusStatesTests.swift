import PostioFFI
import Testing

@testable import PostioKit

/// What the app says about its own state (specs/009-focus-macos T099,
/// screens 16 to 19): the banner under the header strip, the toolbar's sync
/// label and the page an empty list shows. The controller words all three
/// (`postio_ui::focus_state`) and says each only when it changes; the Mac
/// holds what was last said and draws it.
@MainActor
@Suite struct FocusStatesTests {
    static let offline = BannerFfi(
        heading: "You're offline",
        sentence: "Everything you do is saved here and syncs when you're back.",
        button: BannerButtonFfi(label: "Retry now", command: "refresh", key: "F5"),
        progress: nil, error: false, account: nil)

    static let signIn = BannerFfi(
        heading: "Can't sign in to imap.mail.example",
        sentence: "The server rejected the password for ada@example.com.",
        button: BannerButtonFfi(label: "Update password\u{2026}", command: "update_credential", key: nil),
        progress: nil, error: true, account: 7)

    static let firstSync = BannerFfi(
        heading: "First sync",
        sentence: "12,408 of 18,204 messages, newest first. You can read and search what's here.",
        button: nil, progress: BannerProgressFfi(done: 12_408, total: 18_204), error: false,
        account: nil)

    static let empty = EmptyPageFfi(
        heading: "Inbox is empty",
        detail: nil,
        nextDigest: "Next digest: Weekly \u{b7} Newsletters, Saturday 16:00",
        shortcuts: [
            EmptyShortcutFfi(key: "g f", words: "186 filtered today", command: "go_to_filtered"),
            EmptyShortcutFfi(key: "g r", words: "archive", command: "go_to_archive"),
            EmptyShortcutFfi(key: "c", words: "compose", command: "compose"),
        ])

    // MARK: what the controller said, held

    @Test func aBannerIsHeldUntilTheControllerTakesItAway() {
        let states = FocusStates()
        #expect(states.apply(.focusBanner(banner: Self.offline)) == .banner)
        #expect(states.banner == Self.offline)
        #expect(states.apply(.focusBanner(banner: nil)) == .banner)
        #expect(states.banner == nil)
    }

    @Test func theSyncLabelIsTheControllersWordsAndMark() {
        let states = FocusStates()
        #expect(states.syncLabel == nil, "nothing is said before the controller speaks")
        #expect(states.apply(.focusSyncLabel(text: "Offline", mark: .offline)) == .syncLabel)
        #expect(states.syncLabel == SyncLabel(text: "Offline", mark: .offline))
    }

    @Test func theEmptyPageReplacesTheListUntilTheListIsBack() {
        let states = FocusStates()
        #expect(states.empty == nil, "the list is what is drawn until told otherwise")
        #expect(states.apply(.focusEmpty(page: Self.empty)) == .empty)
        #expect(states.empty == Self.empty)
        #expect(states.apply(.focusEmpty(page: nil)) == .empty)
        #expect(states.empty == nil)
    }

    @Test func everyOtherEventChangesNothing() {
        let states = FocusStates()
        _ = states.apply(.focusBanner(banner: Self.offline))
        #expect(states.apply(.backfillProgress(account: 1, done: 3, total: 10)) == nil)
        #expect(states.apply(.keymapChanged) == nil)
        #expect(states.banner == Self.offline)
    }

    // MARK: the banner strip

    @Test func theBannerSaysTheControllersWordsWithItsButtonsKeycap() {
        let words = BannerStripWords(Self.offline)
        #expect(words.heading == "You're offline")
        #expect(words.sentence == Self.offline.sentence)
        #expect(words.button?.label == "Retry now")
        #expect(words.button?.command == "refresh")
        // Spelled as every other cap is: a named key is its glyph or name.
        #expect(words.button?.cap == KeyCapSpelling.cap("F5"))
        #expect(words.tone == .plain)
        #expect(words.progress == nil)
    }

    @Test func aRefusedPasswordIsDrawnAsAnErrorAndNamesItsAccount() {
        let words = BannerStripWords(Self.signIn)
        #expect(words.tone == .error)
        #expect(words.account == 7)
        #expect(words.button?.cap == nil, "a button with no key draws no cap")
    }

    @Test func aFirstSyncDrawsHowFarItHasCome() {
        let words = BannerStripWords(Self.firstSync)
        #expect(words.button == nil)
        let progress = try? #require(words.progress)
        #expect(progress.map { abs($0 - 12_408.0 / 18_204.0) < 0.0001 } == true)
    }

    @Test func progressOverNothingIsNoBar() {
        var banner = Self.firstSync
        banner.progress = BannerProgressFfi(done: 0, total: 0)
        #expect(BannerStripWords(banner).progress == nil)
        banner.progress = BannerProgressFfi(done: 30, total: 20)
        #expect(BannerStripWords(banner).progress == 1, "never past the end")
    }

    // MARK: the empty page

    @Test func theEmptyPageDrawsItsShortcutsAsKeycaps() {
        let words = EmptyInboxWords(Self.empty)
        #expect(words.heading == "Inbox is empty")
        #expect(words.lines == [Self.empty.nextDigest!])
        #expect(words.shortcuts.map(\.cap) == ["g f", "g r", "c"])
        #expect(words.shortcuts.map(\.words) == ["186 filtered today", "archive", "compose"])
        #expect(words.shortcuts.map(\.command) == ["go_to_filtered", "go_to_archive", "compose"])
    }

    @Test func theEmptyPagesDetailComesBeforeItsDigest() {
        var page = Self.empty
        page.detail = "Nothing has arrived since the last pass."
        #expect(EmptyInboxWords(page).lines == [page.detail!, page.nextDigest!])
        page.detail = nil
        page.nextDigest = nil
        #expect(EmptyInboxWords(page).lines.isEmpty)
    }

    @Test func aChordInAShortcutIsDrawnAsMenusDrawIt() {
        var page = Self.empty
        page.shortcuts = [EmptyShortcutFfi(key: "cmd+n", words: "compose", command: "compose")]
        #expect(EmptyInboxWords(page).shortcuts.first?.cap == "⌘N")
    }

    // MARK: the toolbar's label

    @Test func theLabelsSymbolFollowsItsMark() {
        #expect(SyncLabel(text: "Synced 16:09", mark: .synced).symbol == "checkmark.circle")
        #expect(SyncLabel(text: "Syncing 1 of 2", mark: .syncing).symbol == "arrow.triangle.2.circlepath")
        #expect(SyncLabel(text: "Offline", mark: .offline).symbol == "wifi.slash")
        #expect(SyncLabel(text: "Sync failed", mark: .failed).symbol == "exclamationmark.triangle")
    }

    @Test func onlyAFailureIsAlarming() {
        #expect(SyncLabel(text: "Sync failed", mark: .failed).isAlarming)
        #expect(!SyncLabel(text: "Offline", mark: .offline).isAlarming)
        #expect(!SyncLabel(text: "Synced 16:09", mark: .synced).isAlarming)
    }
}
