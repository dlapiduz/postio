import AppKit
import PostioFFI
import SwiftUI
import Testing
import WebKit

@testable import PostioAppKit
@testable import PostioKit

@Suite(.serialized) enum ReaderWebViews {}

extension ReaderWebViews {
    @MainActor
    @Suite(
        .enabled("lays out a SwiftUI view, so it needs a window server") {
            await MainActor.run { NSScreen.main != nil }
        }
    )
    struct ReaderSurfacesTests {
        final class Documents: ReaderSource, @unchecked Sendable {
            func readerDocument(
                message: Int64, remote: RemoteImagesFfi, reduced: Bool
            ) -> ReaderDocumentFfi {
                ReaderDocumentFfi(
                    html: "<!doctype html><p>message \(message)</p>", notice: nil, caveat: nil
                )
            }

            func resolveCid(message: Int64, contentId: String) -> InlinePart? { nil }

            func threadDocument(thread: Int64, reduced: [Int64]) -> ThreadDocumentFfi {
                ThreadDocumentFfi(html: "", messages: [])
            }
        }

        final class SlowDocuments: ReaderSource, @unchecked Sendable {
            private let lock = NSLock()
            private var built = 0
            var answered: Bool { lock.withLock { built > 0 } }

            func readerDocument(
                message: Int64, remote: RemoteImagesFfi, reduced: Bool
            ) -> ReaderDocumentFfi {
                Thread.sleep(forTimeInterval: 2)
                lock.withLock { built += 1 }
                return ReaderDocumentFfi(html: "<p>late</p>", notice: nil, caveat: nil)
            }

            func resolveCid(message: Int64, contentId: String) -> InlinePart? { nil }

            func threadDocument(thread: Int64, reduced: [Int64]) -> ThreadDocumentFfi {
                ThreadDocumentFfi(html: "", messages: [])
            }
        }

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

        private func turn() async {
            try? await Task.sleep(for: .milliseconds(10))
        }

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
            // The render outlives the reader on purpose: a render holding its web view kept a
            // content process alive for a page nobody would see.
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
