import AppKit
import PostioFFI
import SwiftUI
import Testing
import WebKit

@testable import PostioKit

/// Every suite that builds a reader's web view, run one at a time.
///
/// The counters `ReaderSurfacesTests` reads are per thread, and every
/// `@MainActor` suite shares the main thread. A web view another suite built
/// while this one was suspended waiting for SwiftUI landed in this one's
/// delta — `ReaderScrollingTests` makes a `PassingWebView` to prove the wheel
/// passes through it, and a full run counted four readers for three messages.
/// `.serialized` on a parent applies to the suites nested in it.
@Suite(.serialized) enum ReaderWebViews {}

/// How many web views reading a conversation holds — counted, not timed
/// (#1586).
///
/// A rendering surface per message is what ADR 0032 measured at thirty
/// content processes for a thirty-message thread. `postio-gtk` has counted
/// its surfaces in `postio_ui::reader::cost` for as long as that has been
/// the concern; the macOS reader counted nothing, so the claim was gated on
/// one platform and unmeasured on the other. SwiftUI is where it matters
/// most: `makeNSView` runs whenever the framework decides an identity
/// changed, and nothing in the type system says how often that is.
///
/// **What is hosted is the real stack and the real reader.** The only stand-in
/// is the document source, because the real one is a session, and a session
/// on this platform is the Keychain. `ExpandedMessage` is not in the loop — it
/// needs a session for its header — and it holds its `ReaderView`
/// unconditionally, so it contributes no identity of its own to count.
///
/// Every number is read through the boundary from the **shared** counters —
/// the ones `postio-gtk`'s reader notes into — so both frontends are measured
/// against one definition of the cost. They are per thread, which is why the
/// suite is `@MainActor`: the reader notes from the main thread, and a read
/// from any other thread would read that thread's zeros.
///
/// Nested under `ReaderWebViews`, beside the one other suite that builds a
/// reader's web view, so the two never interleave.
extension ReaderWebViews {
    @MainActor
    @Suite(
        .enabled("lays out a SwiftUI view, so it needs a window server") {
            await MainActor.run { NSScreen.main != nil }
        }
    )
    struct ReaderSurfacesTests {
        /// Answers every message with a small document and resolves no parts.
        final class Documents: ReaderSource, @unchecked Sendable {
            func readerDocument(
                message: Int64, remote: RemoteImagesFfi, original: Bool
            ) -> ReaderDocumentFfi {
                ReaderDocumentFfi(
                    html: "<!doctype html><p>message \(message)</p>", notice: nil, caveat: nil
                )
            }

            func resolveCid(message: Int64, contentId: String) -> InlinePart? { nil }
        }

        /// Takes its time over every document, the way a large body on a
        /// busy store does — so a render is still in flight when its reader
        /// is let go of.
        final class SlowDocuments: ReaderSource, @unchecked Sendable {
            private let lock = NSLock()
            private var built = 0
            /// Whether a document has been handed back yet.
            var answered: Bool { lock.withLock { built > 0 } }

            func readerDocument(
                message: Int64, remote: RemoteImagesFfi, original: Bool
            ) -> ReaderDocumentFfi {
                Thread.sleep(forTimeInterval: 2)
                lock.withLock { built += 1 }
                return ReaderDocumentFfi(html: "<p>late</p>", notice: nil, caveat: nil)
            }

            func resolveCid(message: Int64, contentId: String) -> InlinePart? { nil }
        }

        private func row(_ id: Int64) -> RowFfi {
            RowFfi(
                id: id, thread: 7, isThread: false, from: "Ada",
                fromAddress: "ada@example.com", initials: "A", subject: "Radon reduction",
                preview: "Snippet", receivedAt: 1_770_000_000 + id, seen: true, flagged: false,
                answered: false, sendState: nil, hasAttachments: false, threadCount: 6,
                participants: ""
            )
        }

        /// A conversation of `expanded.count` messages, its ids starting at
        /// `first`, open where `expanded` says.
        private func thread(_ id: Int64, expanded: [Bool], first: Int64 = 1) -> ConversationFfi {
            ConversationFfi(
                thread: id, subject: "Radon reduction", meta: "",
                rows: expanded.indices.map { row(first + Int64($0)) },
                focus: UInt32(expanded.count - 1), expanded: expanded
            )
        }

        private func conversation(expanded: [Bool]) -> ConversationModel {
            let model = ConversationModel()
            model.show(thread(7, expanded: expanded))
            return model
        }

        /// A window holding the conversation's stack, each open message a real
        /// reader of `height` points.
        private func host(
            _ model: ConversationModel, height: CGFloat = 120, windowHeight: CGFloat = 1600
        ) -> NSWindow {
            let documents = Documents()
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 800, height: windowHeight),
                styleMask: [.titled], backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: ConversationStack(model: model) { _, row in
                    ReaderView(source: documents, message: row.id).frame(height: height)
                }
            )
            window.contentView?.layoutSubtreeIfNeeded()
            return window
        }

        /// Take the conversation down, and wait until everything it built has
        /// gone.
        ///
        /// Every test ends here rather than in a `defer`. The counters are per
        /// thread and never reset, and the main thread is shared with every other
        /// suite: a reader released a moment after its test returned lands in the
        /// *next* test's delta, which is how a run of the whole suite once
        /// counted four readers for three messages.
        private func close(_ window: NSWindow, back to: Int64) async {
            window.contentView = nil
            window.close()
            _ = await eventually { readerSurfacesHeld() <= to }
        }

        /// Give the main thread back for a moment.
        ///
        /// **Suspending, not spinning the run loop**, and the difference is the
        /// whole of why this suite once reported a leak that was not there. A
        /// reader's render resumes on the main actor, and this suite *is* a
        /// main-actor job: spin `RunLoop.main` synchronously from inside it and
        /// the main queue is not drained re-entrantly, so no render ever finishes
        /// — and an unfinished render holds its web view, which reads exactly
        /// like a view nothing released. Suspending lets the render land and the
        /// run loop turn, SwiftUI's layout included.
        private func turn() async {
            try? await Task.sleep(for: .milliseconds(10))
        }

        /// Take turns until `condition` holds, or give up.
        ///
        /// A web view SwiftUI lets go of is released whenever ARC gets round to
        /// it, so a release cannot be asserted the instant the view goes. This
        /// waits on the condition rather than for a length of time: it returns
        /// the moment the count is right, and the limit only bounds a failure.
        private func eventually(
            within limit: TimeInterval = 10, _ condition: () -> Bool
        ) async -> Bool {
            let deadline = Date(timeIntervalSinceNow: limit)
            while !condition() {
                if Date() >= deadline { return false }
                await turn()
            }
            return true
        }

        /// Let SwiftUI and every render finish what they were going to do, so an
        /// assertion that *nothing more* happened is made after they had the
        /// chance to.
        private func settle() async {
            let until = Date(timeIntervalSinceNow: 0.3)
            while Date() < until { await turn() }
        }

        @Test func threeOpenMessagesHoldThreeWebViewsAndNoMore() async throws {
            let held = readerSurfacesHeld()
            let created = readerSurfacesCreated()
            let model = conversation(expanded: [false, false, false, true, true, true])

            let window = host(model)

            try #require(
                await eventually { readerSurfacesHeld() - held == 3 },
                "three open messages hold \(readerSurfacesHeld() - held) web views"
            )
            await settle()
            #expect(
                readerSurfacesHeld() - held == 3,
                "the stack went on making web views after it had drawn: \(readerSurfacesHeld() - held)"
            )
            #expect(
                readerSurfacesCreated() - created == 3,
                "\(readerSurfacesCreated() - created) web views for three messages: SwiftUI rebuilt a reader it had"
            )
            await close(window, back: held)
        }

        @Test func collapsingAndReopeningAMessageBuildsNoSecondReader() async throws {
            // What collapsing does **not** do, and why that is pinned rather than
            // fixed here: a lazy stack keeps the collapsed message's reader,
            // hidden, in a pool it reuses -- so a collapsed message still holds a
            // content process until the conversation changes. Releasing it on
            // hide would rebuild a reader every time one scrolled back into view,
            // which is the flicker ADR 0032 was written about. The one-document
            // pane is the answer to both, and is not built on this platform.
            //
            // What this does pin is the other half of the trade: reopening costs
            // nothing new. A reader rebuilt on every disclosure would be a
            // process started on every click.
            let held = readerSurfacesHeld()
            let model = conversation(expanded: [false, false, false, true, true, true])
            let window = host(model)
            try #require(await eventually { readerSurfacesHeld() - held == 3 })
            let created = readerSurfacesCreated()

            model.toggle(4)
            await settle()
            model.toggle(4)
            await settle()

            #expect(
                readerSurfacesCreated() == created,
                "collapsing and reopening one message built \(readerSurfacesCreated() - created) new readers"
            )
            #expect(readerSurfacesHeld() - held == 3)
            await close(window, back: held)
        }

        @Test func movingToAnotherConversationReleasesTheLastOnesWebViews() async throws {
            // What `j` does all day. The pane is not rebuilt between
            // conversations -- the model is handed a new one -- and a lazy stack
            // does not dismantle what it stops drawing: it hides the platform
            // view and keeps it. Measured before this was fixed, five
            // conversations of three open messages held eighteen web views, and
            // nothing let go of any of them until the pane itself went.
            let held = readerSurfacesHeld()
            let created = readerSurfacesCreated()
            let model = conversation(expanded: [true, true, true])
            let window = host(model)
            try #require(await eventually { readerSurfacesHeld() - held == 3 })

            for next in 1...4 {
                model.show(thread(Int64(7 + next), expanded: [true, true, true], first: Int64(next * 10)))
                // Wait for the new conversation to be drawn -- its three readers
                // made -- before asking what is still held. Asked sooner, the old
                // three answer "three" and the test passes having seen nothing.
                try #require(
                    await eventually { readerSurfacesCreated() - created == UInt64(3 * (next + 1)) },
                    "the next conversation was never drawn"
                )
                #expect(
                    await eventually { readerSurfacesHeld() - held == 3 },
                    "after \(next) more conversations of three the pane holds \(readerSurfacesHeld() - held) web views"
                )
            }
            await close(window, back: held)
        }

        @Test func aReaderLetGoOfMidRenderIsNotKeptAliveByTheRender() async throws {
            // The render outlives the reader here, deliberately: the document
            // takes two seconds and the pane is closed after none. A render
            // that held its web view kept a content process alive for a page
            // nobody would ever see, for as long as the store took -- and on a
            // busy machine that was long enough to read as a leak.
            let held = readerSurfacesHeld()
            let slow = SlowDocuments()
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 400, height: 400),
                styleMask: [.titled], backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: ReaderView(source: slow, message: 1).frame(height: 100)
            )
            window.contentView?.layoutSubtreeIfNeeded()
            try #require(await eventually { readerSurfacesHeld() - held == 1 })

            window.contentView = nil
            window.close()

            #expect(
                await eventually(within: 1) { readerSurfacesHeld() == held },
                "a render still building its document kept the reader it was for"
            )
            #expect(!slow.answered, "the document arrived first, so this proved nothing")
            // Let the late document arrive and find nothing, before the next
            // test starts counting.
            _ = await eventually { slow.answered }
        }

        @Test func closingTheConversationReleasesEveryWebView() async throws {
            let held = readerSurfacesHeld()
            let model = conversation(expanded: [true, true, true])
            let window = host(model)
            try #require(await eventually { readerSurfacesHeld() - held == 3 })

            window.contentView = nil
            window.close()

            #expect(
                await eventually { readerSurfacesHeld() == held },
                "\(readerSurfacesHeld() - held) web views outlived the conversation they drew"
            )
        }

        @Test func openingALongConversationBuildsOnlyTheReadersItCanShow() async throws {
            // ADR 0032's number: a thirty-message thread, thirty processes. The
            // stack is lazy, so an open message nobody has scrolled to has no
            // reader yet -- this is what notices if that ever stops being true.
            //
            // Opening, not reading. Scrolled to the bottom and back, the same
            // conversation holds all thirty: the stack defers the cost rather
            // than bounding it, which is where the Mac is against ADR 0032.
            let held = readerSurfacesHeld()
            let model = conversation(expanded: Array(repeating: true, count: 30))
            let window = host(model, height: 200, windowHeight: 600)

            try #require(
                await eventually { readerSurfacesHeld() - held > 0 },
                "nothing was drawn at all, so a small count would prove nothing"
            )
            await settle()
            let holding = readerSurfacesHeld() - held
            #expect(
                holding <= 8,
                "thirty open messages hold \(holding) web views in a pane that shows three"
            )
            await close(window, back: held)
        }
    }
}
