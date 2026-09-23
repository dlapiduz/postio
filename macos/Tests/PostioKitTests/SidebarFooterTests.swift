import Foundation
import PostioFFI
import Testing

@testable import PostioKit

/// What the sidebar says under the folders.
@Suite struct SidebarFooterTests {
    private func folder(_ id: Int64, syncedAt: Int64?, total: UInt32 = 4) -> MailboxFfi {
        MailboxFfi(
            id: id,
            account: 1,
            parent: nil,
            name: "Inbox",
            role: .inbox,
            unread: 0,
            total: total,
            selectable: true,
            lastSyncedAt: syncedAt,
            special: true,
            flagged: 0,
            snoozed: 0
        )
    }

    private let now = Date(timeIntervalSince1970: 1_770_000_000)

    @Test func theFooterReportsTheNewestPassAcrossTheFolders() {
        // A store syncs folder by folder; the footer is about the store, so
        // an Archive that has not been touched for a week must not make a
        // freshly-synced inbox read as stale.
        let folders = [
            folder(1, syncedAt: 1_770_000_000 - 40),
            folder(2, syncedAt: 1_770_000_000 - 604_800),
        ]

        #expect(
            SidebarFooter.status(mailboxes: folders, offline: false, syncing: false, now: now)
                == "idle · synced 40s"
        )
    }

    @Test func aStoreThatHasNeverSyncedSaysSo() {
        #expect(
            SidebarFooter.status(
                mailboxes: [folder(1, syncedAt: nil, total: 0)],
                offline: false, syncing: false, now: now
            ) == "idle · never synced"
        )
    }

    @Test func aStoreFullOfMailNeverClaimsItHasNeverSynced() {
        // Read off the running application: five thousand archived messages
        // under a footer reading "never synced". Nothing writes the time yet
        // (#1281), and the line says only the half that is true.
        #expect(
            SidebarFooter.status(
                mailboxes: [folder(1, syncedAt: nil, total: 4985)],
                offline: false, syncing: false, now: now
            ) == "idle"
        )
    }

    @Test func noFoldersAtAllIsAlsoNeverSynced() {
        #expect(
            SidebarFooter.status(mailboxes: [], offline: false, syncing: false, now: now)
                == "idle · never synced"
        )
    }

    @Test func offlineOutranksEverythingElseOnTheLine() {
        #expect(
            SidebarFooter.status(
                mailboxes: [folder(1, syncedAt: 1_770_000_000 - 40)],
                offline: true, syncing: true, now: now
            ) == "offline"
        )
    }

    @Test func aClockThatWentBackwardsReadsAsJustNow() {
        // A machine that slept, or an NTP correction: the last pass appears
        // to be in the future, and "synced -3s" is a bug report on screen.
        #expect(
            SidebarFooter.status(
                mailboxes: [folder(1, syncedAt: 1_770_000_030)],
                offline: false, syncing: false, now: now
            ) == "idle · synced 0s"
        )
    }

    @Test func theDotIsRestingOnlyWhenNothingIsHappening() {
        #expect(SidebarFooter.isResting(offline: false, syncing: false))
        #expect(!SidebarFooter.isResting(offline: true, syncing: false))
        #expect(!SidebarFooter.isResting(offline: false, syncing: true))
    }
    // -- an account that cannot sign in (#1585) ---------------------------

    @Test func aFailingAccountSaysWhyRatherThanHowLongAgoItWorked() {
        // "idle · synced 40s" over an account whose password has expired, and
        // the longer it goes the more settled it looks. The reason wins —
        // which is what `postio_ui::status` has said from the start about the
        // two-line form GTK draws, and what the one-line form had no way to
        // express.
        let line = SidebarFooter.status(
            mailboxes: [folder(1, syncedAt: 1_770_000_000)],
            offline: false,
            syncing: false,
            failure: .auth,
            now: Date(timeIntervalSince1970: 1_770_000_040)
        )
        #expect(!line.contains("synced 40s"), "it told them when it last worked: \(line)")
        #expect(!line.contains("idle"), "nothing about this is idle: \(line)")
        #expect(line == failureSentence(reason: .auth))
    }

    @Test func offlineStillOutranksAFailure() {
        // Offline is the machine's and a failure is the account's: with no
        // connection at all, "the server refused" is a claim about a
        // conversation that did not happen.
        let line = SidebarFooter.status(
            mailboxes: [folder(1, syncedAt: nil)],
            offline: true,
            syncing: false,
            failure: .server,
            now: Date()
        )
        #expect(line == "offline")
    }

    @Test func anAccountThatIsFineSaysWhatItSaidBefore() {
        // The ordinary line is unchanged; `failure` defaults to none.
        let line = SidebarFooter.status(
            mailboxes: [folder(1, syncedAt: 1_770_000_000)],
            offline: false,
            syncing: false,
            now: Date(timeIntervalSince1970: 1_770_000_040)
        )
        #expect(line == "idle · synced 40s")
    }

    @Test func theDotIsNotRestingWhileSomethingIsWrong() {
        // The dot is the glanceable half: filled means "something is
        // happening or wrong", and a failing account is the second.
        #expect(!SidebarFooter.isResting(offline: false, syncing: false, failure: .auth))
        #expect(SidebarFooter.isResting(offline: false, syncing: false, failure: nil))
    }

}
