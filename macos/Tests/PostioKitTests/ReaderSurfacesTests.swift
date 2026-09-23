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

/// How many web views the single-message reader holds -- counted, not timed
/// (#1586).
///
/// The conversation is one document now, and `ThreadDocumentTests` counts
/// it. What is here is the other reader: a message the store has not
/// threaded, drawn by `ReaderView` on its own. Every number is read through
/// the boundary from the **shared** counters -- the ones `postio-gtk`'s reader
/// notes into -- and they are per thread, which is why the suite is
/// `@MainActor`: the reader notes from the main thread.
///
/// Nested under `ReaderWebViews`, beside the other suites that build a
/// reader's web view, so none of them interleave.
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

            func threadDocument(thread: Int64, originals: [Int64]) -> ThreadDocumentFfi {
                ThreadDocumentFfi(html: "", messages: [], rail: [])
            }
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

            func threadDocument(thread: Int64, originals: [Int64]) -> ThreadDocumentFfi {
                ThreadDocumentFfi(html: "", messages: [], rail: [])
            }
        }

        /// A window holding one message's reader.
        private func host(_ source: any ReaderSource, message: Int64) -> NSWindow {
            let window = NSWindow(
                contentRect: NSRect(x: 0, y: 0, width: 400, height: 400),
                styleMask: [.titled], backing: .buffered, defer: false
            )
            window.isReleasedWhenClosed = false
            window.contentView = NSHostingView(
                rootView: ReaderView(source: source, message: message).frame(height: 100)
            )
            window.contentView?.layoutSubtreeIfNeeded()
            return window
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

        @Test func aReaderLetGoOfMidRenderIsNotKeptAliveByTheRender() async throws {
            // The render outlives the reader here, deliberately: the document
            // takes two seconds and the pane is closed after none. A render
            // that held its web view kept a content process alive for a page
            // nobody would ever see, for as long as the store took -- and on a
            // busy machine that was long enough to read as a leak.
            let held = readerSurfacesHeld()
            let slow = SlowDocuments()
            let window = host(slow, message: 1)
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


        @Test func closingTheReaderReleasesItsWebView() async throws {
            let held = readerSurfacesHeld()
            let created = readerSurfacesCreated()
            let window = host(Documents(), message: 1)
            try #require(await eventually { readerSurfacesHeld() - held == 1 })
            #expect(readerSurfacesCreated() - created == 1, "one message, one reader")

            window.contentView = nil
            window.close()

            #expect(
                await eventually { readerSurfacesHeld() == held },
                "the reader outlived the pane it drew"
            )
        }
    }
}
