import AppKit
import PostioFFI
import PostioAppKit
import PostioKit
import SwiftUI

/// The main window: Focus's inbox (specs/009-focus-macos US1, screens 01
/// and 02).
///
/// A unified toolbar -- compose on the left, the sync label and the search
/// field on the right -- then the header strip, then the list. No sidebar,
/// no reading pane: an email opens in its own window (FR-011, later), and
/// the list is the whole of the window.
///
/// The toolbar is AppKit's (`MainToolbar`), because `NSSearchToolbarItem`
/// is the field the command bar will drop from (FR-013); the strip is
/// SwiftUI over `HeaderStripWords`; the list is `FocusListTable` (R10).
struct MainWindow: View {
    let engine: Engine
    /// Only a view can open a window, so this is where a `settings` or
    /// `compose` command becomes one (#1261).
    @Environment(\.openWindow) private var openWindow

    var body: some View {
        Group {
            // A fresh install is the wizard, not an empty inbox: with no
            // account there is nothing for the list to show (canvas 09).
            if let firstRun = engine.firstRun, engine.accounts.isEmpty {
                FirstRunView(
                    model: firstRun,
                    otherWays: { engine.run(Intercepted.addAccount) },
                    finished: { engine.settingsActions.accountAdded?() }
                )
            } else {
                inbox
            }
        }
        // The inbox fills the window: without a size of its own the table
        // reports almost none, and the window shrank to a strip of rows.
        .frame(minWidth: 800, idealWidth: 1440, minHeight: 500, idealHeight: 900)
        // A count rather than a flag: two `⌘,` presses are two openings, and
        // `onChange` compares values (see `WindowRequest`).
        .onChange(of: engine.settingsWindow) { _, request in
            guard request.wasRaised else { return }
            openWindow(id: request.id)
        }
        // A compose window per draft: the store holds the draft, this is
        // what can open a window for it.
        .onChange(of: engine.compose.request) { _, request in
            guard request.wasRaised, let draft = engine.compose.requested else { return }
            openWindow(id: WindowId.compose, value: draft)
        }
    }

    private var inbox: some View {
        VStack(spacing: 0) {
            // Inbox ▾ is what the folders popover hangs from (T086).
            HeaderStrip(
                words: engine.stripWords,
                placeAnchor: PlacesAnchor { engine.placesAnchor = $0 }
            ) { engine.run($0) }
            // First sync, offline, a refused password (screens 17 to 19):
            // one full-width strip under the header strip, as the
            // controller words it. Never in the way: the list stays live.
            if let banner = engine.states.banner {
                BannerStrip(words: BannerStripWords(banner)) { engine.run($0) }
                    .transition(.opacity)
            }
            list
            // While anything is selected (screen 01): the count, the verbs
            // with their keys, and the selection's own keys.
            if let words = engine.actionBarWords {
                ActionBar(words: words) { engine.run($0) }
                    .transition(.opacity)
            }
        }
        // The command bar is a panel dropping from the toolbar's field
        // (`CommandBarPanel`, T085), a child window rather than an overlay.
        .background(MainToolbarInstaller(engine: engine))
        // "Update password…" from the sign-in banner (screen 19).
        .sheet(isPresented: Binding(
            get: { engine.bannerRepairAccount != nil },
            set: { if !$0 { engine.bannerRepair.cancel() } }
        )) {
            if let account = engine.bannerRepairAccount {
                PasswordSheet(account: account, repair: engine.bannerRepair, session: engine.session)
            }
        }
        .sheet(isPresented: Binding(
            get: { engine.showingCheatSheet },
            set: { engine.showingCheatSheet = $0 }
        )) {
            if let session = engine.session {
                CheatSheet(session: session, context: .list, dismiss: { engine.dismissOverlays() })
            }
        }
        // `PRODUCT.md` §18: ≤100 ms or absent, and Reduce Motion is honoured.
        .animation(.easeOut(duration: Motion.current), value: engine.pendingChord)
        .animation(.easeOut(duration: Motion.current), value: engine.noticeToken)
        .animation(.easeOut(duration: Motion.current), value: engine.actionBarWords != nil)
        .animation(.easeOut(duration: Motion.current), value: engine.states.banner)
        .animation(.easeOut(duration: Motion.current), value: engine.focus.toastToken)
        .animation(.easeOut(duration: Motion.current), value: engine.focus.toast == nil)
        .overlay(alignment: .bottomLeading) { NoticeBanner(engine: engine) }
        // What a verb did, with Undo while the stack can take it back: the
        // controller's toast, as the pill at the bottom centre (T093,
        // screen 15). Above the action bar when there is one.
        .overlay(alignment: .bottom) {
            if let toast = engine.focus.toast {
                UndoPill(
                    toast: toast, token: engine.focus.toastToken, undoCap: engine.undoCap,
                    undo: { engine.run(Notice.undoCommand) },
                    dismiss: { engine.focus.dismissToast(token: $0) }
                )
                .padding(.bottom, engine.actionBarWords == nil ? 0 : ActionBar.height)
            }
        }
        .overlay(alignment: .bottomTrailing) {
            // A half-typed sequence, shown while it waits: `g` on its own is
            // otherwise a second of the application ignoring a key.
            if let pending = engine.pendingChord {
                Text(pending)
                    .font(.system(.body, design: .monospaced))
                    .padding(.horizontal, 8)
                    .padding(.vertical, 4)
                    .background(.regularMaterial, in: .rect(cornerRadius: 6))
                    .padding(12)
                    .transition(.opacity)
                    .accessibilityLabel("Waiting for the rest of \(pending)")
            }
        }
    }

    @ViewBuilder
    private var list: some View {
        switch engine.state {
        case .opening:
            // The window the Keychain's prompt appears over (#1146).
            ContentUnavailableView {
                Label("Unlocking your mail", systemImage: "lock")
            } description: {
                Text("Postio is asking the Keychain for this store's key.")
            }
        case .open:
            if let page = engine.states.empty {
                // Empty is a state, not a blank (screen 16): the controller's
                // page, with its next digest and the shortcuts that lead
                // somewhere, in the list's place until it says the list is
                // back.
                EmptyInbox(words: EmptyInboxWords(page)) { engine.run($0) }
            } else if let table = engine.focusTable {
                FocusListView(table: table)
            }
        case let .refused(model):
            // The store would not open: why, and the way forward (T100).
            StoreRefusalPage(model: model)
        }
    }
}

/// A failure Postio has to report: a sentence where the eye already is,
/// gone on its own. `Notice` decides; this draws. Completions, undos and
/// refusals are the controller's toast in this window (`UndoPill`),
/// so only `Notice.shownBesideFocusToast` reaches here.
private struct NoticeBanner: View {
    let engine: Engine

    var body: some View {
        if let notice = engine.notice {
            HStack(spacing: 8) {
                Image(systemName: notice.isAlarming ? "exclamationmark.triangle.fill" : "checkmark.circle")
                    .foregroundStyle(notice.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
                Text(notice.message).lineLimit(2)
                if notice.offersUndo {
                    // The registry's `undo`, so this button and the key are
                    // one command with one stack.
                    Button("Undo") {
                        engine.run(Notice.undoCommand)
                        engine.dismissNotice()
                    }
                    .buttonStyle(.link)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(.regularMaterial, in: .rect(cornerRadius: 8))
            .padding(12)
            .transition(.opacity)
            .accessibilityElement(children: .combine)
            .accessibilityLabel(notice.message)
            .accessibilityAddTraits(.updatesFrequently)
            .task(id: engine.noticeToken) {
                try? await Task.sleep(nanoseconds: UInt64(notice.seconds * 1_000_000_000))
                guard !Task.isCancelled else { return }
                engine.dismissNotice()
            }
        }
    }
}

// MARK: - the toolbar

/// Puts `MainToolbar` on the window this view is in, once it is in one.
///
/// SwiftUI has no `NSSearchToolbarItem`, and the command bar has to drop
/// from a real one (FR-013), so the toolbar is AppKit's. Nothing in the
/// SwiftUI tree declares a `.toolbar`, so SwiftUI has none of its own to
/// put back over this one.
private struct MainToolbarInstaller: NSViewRepresentable {
    let engine: Engine

    final class Coordinator {
        var toolbar: MainToolbar?
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSView(context: Context) -> NSView {
        let view = NSView(frame: .zero)
        let coordinator = context.coordinator
        let engine = engine
        // `window` is nil until the view joins a hierarchy. One hop.
        DispatchQueue.main.async {
            guard let window = view.window else { return }
            let toolbar = coordinator.toolbar ?? MainToolbar(engine: engine)
            coordinator.toolbar = toolbar
            toolbar.install(in: window)
        }
        return view
    }

    func updateNSView(_: NSView, context _: Context) {}
}

/// The unified toolbar (FR-010): compose on the left, a flexible space,
/// the sync label, and the search field with its ⌘K keycap.
@MainActor
final class MainToolbar: NSObject, NSToolbarDelegate, NSSearchFieldDelegate {
    private let engine: Engine
    private let toolbar = NSToolbar(identifier: "PostioFocusMain")
    private weak var field: BarSearchField?
    private weak var window: NSWindow?
    private let cap = KeyCapView("")

    static let compose = NSToolbarItem.Identifier("postio.compose")
    static let sync = NSToolbarItem.Identifier("postio.sync")
    static let search = NSToolbarItem.Identifier("postio.search")

    init(engine: Engine) {
        self.engine = engine
        super.init()
        toolbar.delegate = self
        toolbar.displayMode = .iconOnly
        toolbar.allowsUserCustomization = false
    }

    func install(in window: NSWindow) {
        guard self.window !== window else { return }
        self.window = window
        window.toolbarStyle = .unified
        window.titleVisibility = .hidden
        if window.toolbar !== toolbar { window.toolbar = toolbar }
        // SwiftUI rebuilds what it owns when a scene updates; if that ever
        // takes the toolbar back, put this one back rather than lose search.
        NotificationCenter.default.addObserver(
            forName: NSWindow.didUpdateNotification, object: window, queue: .main
        ) { [weak self] note in
            guard let window = note.object as? NSWindow else { return }
            MainActor.assumeIsolated {
                guard let self, window.toolbar !== self.toolbar else { return }
                window.toolbar = self.toolbar
            }
        }
        // The field is the command bar's (T085): the engine puts the
        // controller's words in it and gives it the keyboard.
        engine.searchField = field
        engine.keycapsChanged = { [weak self] in self?.respell() }
        respell()
    }

    // MARK: NSToolbarDelegate

    func toolbarDefaultItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        [Self.compose, .flexibleSpace, Self.sync, Self.search]
    }

    func toolbarAllowedItemIdentifiers(_ toolbar: NSToolbar) -> [NSToolbarItem.Identifier] {
        toolbarDefaultItemIdentifiers(toolbar)
    }

    func toolbar(
        _ toolbar: NSToolbar,
        itemForItemIdentifier identifier: NSToolbarItem.Identifier,
        willBeInsertedIntoToolbar flag: Bool
    ) -> NSToolbarItem? {
        switch identifier {
        case Self.compose:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.image = NSImage(systemSymbolName: "square.and.pencil", accessibilityDescription: "Compose")
            item.label = "Compose"
            item.target = self
            item.action = #selector(compose)
            item.toolTip = composeTip()
            return item
        case Self.sync:
            let item = NSToolbarItem(itemIdentifier: identifier)
            item.view = NSHostingView(rootView: SyncLabelView(engine: engine))
            item.label = "Sync"
            return item
        case Self.search:
            let item = NSSearchToolbarItem(itemIdentifier: identifier)
            // A click into it opens the bar, as `/` does.
            let field = BarSearchField()
            field.onFocus = { [weak self] in self?.engine.searchFieldFocused() }
            item.searchField = field
            item.preferredWidthForSearchField = 320
            field.placeholderString = "Search mail or run a command"
            field.delegate = self
            field.sendsSearchStringImmediately = false
            field.sendsWholeSearchString = true
            cap.translatesAutoresizingMaskIntoConstraints = false
            field.addSubview(cap)
            NSLayoutConstraint.activate([
                cap.trailingAnchor.constraint(equalTo: field.trailingAnchor, constant: -6),
                cap.centerYAnchor.constraint(equalTo: field.centerYAnchor),
            ])
            self.field = field
            engine.searchField = field
            respell()
            return item
        default:
            return nil
        }
    }

    @objc private func compose() {
        engine.run(Intercepted.compose)
    }

    /// "Compose · c", as `postio_ui::focus_row::compose_tooltip` says it.
    private func composeTip() -> String {
        KeyCapSpelling.cap(engine.session?.binding(for: Intercepted.compose))
            .map { "Compose \u{b7} \($0)" } ?? "Compose"
    }

    /// The keycap in the field and the compose tip, from the bindings in
    /// force: at install, when a session opens, and when `[keys]` changes.
    private func respell() {
        let palette = KeyCapSpelling.cap(engine.session?.binding(for: BarCommand.palette))
        cap.text = palette ?? ""
        cap.isHidden = palette == nil || !(field?.stringValue.isEmpty ?? true)
        for item in toolbar.items where item.itemIdentifier == Self.compose {
            item.toolTip = composeTip()
        }
    }

    // MARK: NSSearchFieldDelegate

    func controlTextDidEndEditing(_ obj: Notification) {
        engine.searchFieldLeft()
    }

    func controlTextDidChange(_ obj: Notification) {
        let text = field?.stringValue ?? ""
        cap.isHidden = cap.text.isEmpty || !text.isEmpty
        engine.searchFieldTyped(text)
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        engine.searchFieldCommand(selector)
    }
}

/// The sync label: a symbol and the controller's words ("Synced 16:09",
/// "Syncing 12,408 of 18,204", "Offline", "Sync failed"), as
/// `FocusSyncLabel` last said them. Nothing until it has said anything: the
/// words are `postio_ui::focus_state::sync_label`'s, and a Swift copy of
/// them is the drift the FFI event exists to end.
private struct SyncLabelView: View {
    let engine: Engine

    var body: some View {
        if let label = engine.states.syncLabel {
            HStack(spacing: 5) {
                Image(systemName: label.symbol).font(.system(size: 11))
                Text(label.text).font(.system(size: 12).monospacedDigit())
            }
            .foregroundStyle(label.isAlarming ? AnyShapeStyle(.red) : AnyShapeStyle(.secondary))
            .fixedSize()
            .accessibilityElement(children: .combine)
        }
    }
}
