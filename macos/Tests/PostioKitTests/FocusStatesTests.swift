import PostioFFI
import Testing

@testable import PostioKit

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
            EmptyShortcutFfi(key: "g a", words: "archive", command: "go_to_archive"),
            EmptyShortcutFfi(key: "c", words: "compose", command: "compose"),
        ])

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

    @Test func theBannerSaysTheControllersWordsWithItsButtonsKeycap() {
        let words = BannerStripWords(Self.offline)
        #expect(words.heading == "You're offline")
        #expect(words.sentence == Self.offline.sentence)
        #expect(words.button?.label == "Retry now")
        #expect(words.button?.command == "refresh")
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

    @Test func theEmptyPageDrawsItsShortcutsAsKeycaps() {
        let words = EmptyInboxWords(Self.empty)
        #expect(words.heading == "Inbox is empty")
        #expect(words.lines == [Self.empty.nextDigest!])
        #expect(words.shortcuts.map(\.cap) == ["g f", "g a", "c"])
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

@MainActor
@Suite struct ScopeFooterTests {
    static let child = FooterViewFfi(
        title: "Message 4 of 6 \u{b7} Ben Adeyemi",
        keys: [
            FooterKeyFfi(cap: "j k", label: "move"),
            FooterKeyFfi(cap: "e", label: "reply to this"),
            FooterKeyFfi(cap: "\u{21b5}", label: "open here"),
        ],
        note: "e, b, l, v act on the thread")

    @Test func theFooterHoldsTheTitleTheKeysAndTheNote() {
        let states = FocusStates()
        #expect(states.footer == nil, "nothing is drawn before the controller says")

        #expect(states.apply(.focusFooter(view: Self.child)) == .footer)

        #expect(states.footer?.title == "Message 4 of 6 \u{b7} Ben Adeyemi")
        #expect(states.footer?.keys.map(\.cap) == ["j k", "e", "\u{21b5}"])
        #expect(states.footer?.keys.map(\.label) == ["move", "reply to this", "open here"])
        #expect(states.footer?.note?.hasSuffix(" act on the thread") == true)
    }

    @Test func aRowThatStandsForOneMessageHasNoNote() {
        let states = FocusStates()
        _ = states.apply(.focusFooter(view: FooterViewFfi(title: "Atlas staffing plan for Q4", keys: [], note: nil)))
        #expect(states.footer?.note == nil)
        let words = ScopeFooterWords(states.footer!)
        #expect(words.hints.isEmpty)
        #expect(words.note == nil)
    }

    @Test func theWordsTurnTheKeysIntoHints() {
        let words = ScopeFooterWords(Self.child)
        #expect(words.title == "Message 4 of 6 \u{b7} Ben Adeyemi")
        #expect(words.hints.map(\.cap) == ["j k", "e", "\u{21b5}"])
        #expect(words.hints.map(\.label) == ["move", "reply to this", "open here"])
    }
}
