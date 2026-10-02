import PostioFFI
import Testing

@testable import PostioKit

/// Which surface a keystroke resolves as when the keyboard is not in the
/// main window.
///
/// The key monitor is a *local* monitor: it sees every key press in the
/// application, including the ones typed into a compose window. What it asks
/// for the context, though, was the main window's focused pane and nothing
/// else — so `Context::Composer` was never once the answer, and two things
/// followed from that. Every composer binding (`send`, `save_draft`,
/// `discard_draft`, `attach_file`, `bold`, `insert_link`) resolved to
/// nothing, because those commands are only available in a context the
/// resolver was never given. And the main window's own bindings kept
/// resolving while a message was being written: `a` in a compose window
/// archived whatever the list cursor happened to be on.
///
/// The rule is that **the window decides first**. A pane only gets a say when
/// the keyboard is in the window that has panes.
@Suite struct KeyWindowContextTests {
    @Test func aComposeWindowResolvesAsTheComposerWhateverThePaneWas() {
        // The pane is deliberately every value it can hold: none of them may
        // reach the answer while a compose window has the keyboard.
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
        // Overlays are the main window's context too — the palette and the
        // search bar are drawn inside it, and the engine already sets those.
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .palette) == .palette)
        #expect(KeyboardContext.resolving(keyWindow: .main, mainWindow: .search) == .search)
    }

    @Test func theSettingsWindowIsNotTheListEither() {
        // Settings has its own context in the registry (`Context::Accounts`
        // is the pane inside it that has keys). What matters here is the
        // negative: `d` in the settings window must not delete a message.
        #expect(KeyboardContext.resolving(keyWindow: .settings, mainWindow: .list) != .list)
    }

    /// The commands this was costing, held against the registry rather than
    /// listed here — a command that gains or loses `Composer` must move this
    /// test, not silently pass it.
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
