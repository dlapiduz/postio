import PostioFFI
import Testing

@testable import PostioKit

@MainActor
@Suite struct ComposeCommandsTests {
    private func model(rich: Bool = true) -> ComposeModel {
        ComposeModel(
            id: 1,
            draft: DraftFfi(
                id: 0, account: 1, kind: .new,
                from: "Mara Ostwald <mara@example.com>",
                to: "", cc: "", bcc: "", subject: "", body: "",
                bodyHtml: nil, rich: rich, inReplyTo: nil,
                path: "/tmp/mail", attachments: []
            )
        )
    }

    @Test func aMarkReachesTheDocument() {
        let composer = model()
        for mark in ["bold", "italic", "heading_1", "bullet_list", "numbered_list", "quote_block"] {
            let before = composer.markRequest?.serial ?? 0
            #expect(ComposeCommands.run(mark, on: composer, through: nil))
            #expect(composer.markRequest?.command == mark)
            #expect(
                (composer.markRequest?.serial ?? 0) > before,
                "\(mark) did not ask again"
            )
        }
    }

    @Test func aMarkOnAPlainDraftIsStillClaimedAndDoesNothing() {
        let composer = model(rich: false)
        #expect(ComposeCommands.run("bold", on: composer, through: nil))
        #expect(composer.markRequest == nil)
    }

    @Test func insertImageAsksForAPicture() {
        let composer = model()
        #expect(ComposeCommands.run("insert_image", on: composer, through: nil))
        #expect(composer.wantsImage)
    }

    @Test func insertImageOnAPlainDraftSaysWhyNot() {
        // A plain draft has no document for a picture: claimed and said, since a silent key cannot
        // be told from a broken one.
        let composer = model(rich: false)
        #expect(ComposeCommands.run("insert_image", on: composer, through: nil))
        #expect(!composer.wantsImage)
        #expect(composer.status?.contains("Rich") == true, "\(composer.status ?? "nothing said")")
    }

    @Test func aPictureTheBoundaryStoredReachesTheDocumentOnce() {
        // The surface must run the script once: re-run on every SwiftUI update it would insert the
        // picture on every keystroke.
        let composer = model()
        var withPart = composer.draft
        withPart.id = 9
        withPart.attachments = [
            AttachmentFfi(id: 3, filename: "inline-image.png", mimeType: "image/png", size: "12 B")
        ]
        let before = composer.imageRequest?.serial ?? 0

        composer.took(InlineImageFfi(draft: withPart, script: "insert();"))

        #expect(composer.draft.id == 9, "the id the store assigned is kept")
        #expect(composer.attachments.count == 1)
        #expect(composer.imageRequest?.script == "insert();")
        #expect((composer.imageRequest?.serial ?? 0) == before + 1)
    }

    @Test func theCopyFieldsCommandOpensThem() {
        let composer = model()
        #expect(!composer.showsCopyFields)
        #expect(ComposeCommands.run("copy_fields", on: composer, through: nil))
        #expect(composer.showsCopyFields)
    }

    @Test func aCommandTheComposerDoesNotOwnIsLeftAlone() {
        // Claiming `archive` in a compose window would be the bug this layer guards against, in
        // reverse.
        let composer = model()
        #expect(!ComposeCommands.run("archive", on: composer, through: nil))
        #expect(!ComposeCommands.run("next_message", on: composer, through: nil))
    }

    @Test func everyClaimedIdIsAComposerCommand() {
        let specs = Dictionary(
            uniqueKeysWithValues: PostioRegistry.commands.map { ($0.id, $0) }
        )
        for id in ComposeCommands.handled {
            let spec = specs[id]
            #expect(spec != nil, "`\(id)` is claimed and is not a command")
            #expect(
                spec?.contexts.contains(.composer) == true,
                "`\(id)` is claimed by the composer and the registry does not put it there"
            )
        }
    }
}
