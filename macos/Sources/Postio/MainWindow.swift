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
            HeaderStrip(words: engine.stripWords) { engine.run($0) }
            list
        }
        .background(MainToolbarInstaller(engine: engine))
        // The commands the search field offers while it holds `>`, just
        // under the field. The keyboard stays in the field; the command bar
        // proper (a panel dropping from it, FR-013) replaces this later.
        .overlay(alignment: .topTrailing) {
            if engine.finding != nil {
                FinderResults(
                    rows: engine.finderAnswer.rows,
                    empty: engine.finderAnswer.empty,
                    highlighted: engine.finderBox.highlighted,
                    pick: { engine.pick($0) }
                )
                .padding(.top, 4)
                .padding(.trailing, 12)
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
        .animation(.easeOut(duration: Motion.current), value: engine.finding != nil)
        .animation(.easeOut(duration: Motion.current), value: engine.noticeToken)
        .overlay(alignment: .bottomLeading) { NoticeBanner(engine: engine) }
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
            if let table = engine.focusTable, !(engine.focusListed && engine.focusCount == 0) {
                FocusListView(table: table)
            } else {
                // Empty is a state, not a blank. Screen 16's empty inbox, with
                // its next digest and shortcuts, comes with the app states.
                ContentUnavailableView("Inbox is empty", systemImage: "tray")
            }
        case let .unavailable(reason):
            ContentUnavailableView {
                Label("The engine did not open", systemImage: "exclamationmark.triangle")
            } description: {
                Text(reason)
            }
        }
    }
}

/// What Postio last said back about something you asked it to do: a
/// sentence where the eye already is, gone on its own, with an Undo when
/// there is something to take back. `Notice` decides; this draws. (The
/// bottom-centre undo pill of screen 15 replaces it.)
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
    private weak var field: NSSearchField?
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
        // The engine asks the field for text (⌘K types `>`) and for the
        // keyboard (`/`); the field is AppKit's, so these are its hands.
        engine.fieldText = { [weak self] text in
            self?.field?.stringValue = text
            self?.fieldChanged()
        }
        engine.focusField = { [weak self] in
            guard let field = self?.field else { return }
            field.window?.makeFirstResponder(field)
        }
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
            item.preferredWidthForSearchField = 320
            let field = item.searchField
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
        let palette = KeyCapSpelling.cap(engine.session?.binding(for: Intercepted.palette))
        cap.text = palette ?? ""
        cap.isHidden = palette == nil || !(field?.stringValue.isEmpty ?? true)
        for item in toolbar.items where item.itemIdentifier == Self.compose {
            item.toolTip = composeTip()
        }
    }

    // MARK: NSSearchFieldDelegate

    func controlTextDidBeginEditing(_ obj: Notification) {
        engine.showingSearch = true
    }

    func controlTextDidEndEditing(_ obj: Notification) {
        engine.showingSearch = false
    }

    func controlTextDidChange(_ obj: Notification) {
        fieldChanged()
    }

    private func fieldChanged() {
        let text = field?.stringValue ?? ""
        cap.isHidden = cap.text.isEmpty || !text.isEmpty
        engine.findingChanged(FinderBox.asking(in: text))
    }

    func control(_ control: NSControl, textView: NSTextView, doCommandBy selector: Selector) -> Bool {
        guard engine.finding != nil else { return false }
        switch selector {
        case #selector(NSResponder.moveUp(_:)):
            engine.moveFinder(by: -1)
        case #selector(NSResponder.moveDown(_:)):
            engine.moveFinder(by: 1)
        case #selector(NSResponder.insertNewline(_:)):
            engine.pickHighlighted()
        default:
            return false
        }
        return true
    }
}

/// The sync label: a symbol and "Synced 16:09", redrawn as it ages.
private struct SyncLabelView: View {
    let engine: Engine

    var body: some View {
        TimelineView(.periodic(from: .now, by: 30)) { _ in
            let label = engine.syncLabel
            HStack(spacing: 5) {
                Image(systemName: label.symbol).font(.system(size: 11))
                Text(label.text).font(.system(size: 12).monospacedDigit())
            }
            .foregroundStyle(engine.failure == nil ? AnyShapeStyle(.secondary) : AnyShapeStyle(.red))
            .fixedSize()
            .accessibilityElement(children: .combine)
        }
    }
}
