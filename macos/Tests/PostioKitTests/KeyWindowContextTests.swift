import PostioFFI
import Testing

@testable import PostioKit

@Suite struct KeyWindowContextTests {
    @Test func aComposeWindowResolvesAsTheComposerWhateverThePaneWas() {
        // None of the pane values may reach the answer while a compose window has the keyboard.
        for pane in [UiContext.list, .sidebar, .reader, .conversation, .search] {
            #expect(
                KeyboardContext.resolving(keyWindow: .compose, mainWindow: pane) == .composer,
                "a key typed into a compose window resolved as \(pane)"
            )
        }
    }

    @Test func theMainWindowStillAnswersWithItsOwnPane() {
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .list) == .list)
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .sidebar) == .sidebar)
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .reader) == .reader)
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .palette) == .palette)
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .search) == .search)
    }

    @Test func theMessageWindowResolvesAsTheReader() {
        for pane in [UiContext.list, .search] {
            #expect(KeyboardContext.resolving(keyWindow: .message, mainWindow: pane) == .reader)
        }
    }

    @Test func theSettingsWindowIsNotTheListEither() {
        // `d` in the settings window must not delete a message.
        #expect(KeyboardContext.resolving(keyWindow: .settings, mainWindow: .list) != .list)
    }

    @Test func everyComposerOnlyCommandIsReachableOnceTheContextIsRight() {
        let composerOnly = PostioRegistry.commands.filter { spec in
            spec.contexts.contains(.composer) && !spec.contexts.contains(.list)
        }
        #expect(
            !composerOnly.isEmpty,
            "the registry has no composer-only commands, so this test proves nothing"
        )
        let context = KeyboardContext.resolving(keyWindow: .compose, mainWindow: .list)
        for spec in composerOnly {
            #expect(
                spec.contexts.contains(context),
                "\(spec.id) is unreachable: a compose window resolves as \(context)"
            )
        }
    }
}
