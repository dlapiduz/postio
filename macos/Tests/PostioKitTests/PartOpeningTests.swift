import PostioFFI
import Testing
@testable import PostioKit

@Suite struct PartOpeningTests {
    private func part(
        mime: String,
        container: Bool = false,
        downloaded: Bool = true,
        previewable: Bool = false
    ) -> PartFfi {
        PartFfi(
            partId: "2",
            depth: 1,
            prefix: "  ├ ",
            mimeType: mime,
            filename: nil,
            label: mime,
            saveName: "part",
            size: 10,
            detail: mime,
            spoken: mime,
            downloaded: downloaded,
            isContainer: container,
            inline: false,
            contentId: nil,
            previewable: previewable
        )
    }

    @Test func aPartPostioCanDrawIsDrawnInsidePostio() {
        // The default matters: inside Postio a document cannot fetch, launch
        // or phone home. Handing it out is the exception, not the rule.
        #expect(PartOpening.of(part(mime: "image/png", previewable: true)) == .preview)
    }

    @Test func anythingElseGoesToWhateverOpensIt() {
        // The boundary's own words about `previewable`: everything else is
        // bytes the application has no business interpreting.
        #expect(PartOpening.of(part(mime: "application/zip")) == .desktop)
    }

    @Test func aContainerHasNothingToOpen() {
        // `multipart/mixed` is shape, not content.
        #expect(PartOpening.of(part(mime: "multipart/mixed", container: true)) == .nothing)
    }

    @Test func aPartWhoseBytesHaveNotArrivedOpensNothing() {
        // Going and getting them here would be the reader reaching the
        // network on a keystroke — the thing the whole reader is built not
        // to do. The ordinary state of an attachment on a described message.
        #expect(PartOpening.of(part(mime: "image/png", downloaded: false, previewable: true)) == .nothing)
    }

    @Test func anEmptyPanelOpensNothing() {
        #expect(PartOpening.of(nil) == .nothing)
    }
}
