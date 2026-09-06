import PostioFFI
import SwiftUI

/// Canvas 3f, as a macOS window.
///
/// The frame is the contract and the panes are the easy part: navigation down
/// the side that jumps to a section rather than opening a sub-screen, the
/// validity line along the foot where a dialog would put OK and Cancel, and
/// no staging copy anywhere — a change lands when you make it.
///
/// ADR 0029 is why this is a window rather than the in-window pane GTK shows.
public struct SettingsPaneView: View {
    @Bindable private var store: SettingsStore

    public init(store: SettingsStore) {
        self.store = store
    }

    public var body: some View {
        NavigationSplitView {
            List(store.sections, id: \.key, selection: $store.selected) { section in
                Text(section.title).tag(section.key)
            }
            .navigationSplitViewColumnWidth(min: 160, ideal: 180, max: 220)
        } detail: {
            VStack(alignment: .leading, spacing: 0) {
                pane
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
                Divider()
                footer
            }
        }
        .frame(minWidth: 620, minHeight: 420)
    }

    @ViewBuilder private var pane: some View {
        if store.selected == "ui" {
            appearance
        } else {
            unbuilt
        }
    }

    /// The `[ui]` table, as fields.
    ///
    /// Disabled wholesale when the file will not parse: the controls would
    /// otherwise show defaults that are not the user's, and touching one would
    /// save them over the file they opened this window to fix.
    @ViewBuilder private var appearance: some View {
        if let current = store.appearance {
            Form {
                Section {
                    Picker("Density", selection: binding(current, \.density)) {
                        Text("Airy").tag(DensityFfi.airy)
                        Text("Comfortable").tag(DensityFfi.comfortable)
                        Text("Compact").tag(DensityFfi.compact)
                    }
                    Picker("Theme", selection: binding(current, \.theme)) {
                        Text("System").tag(ThemeFfi.system)
                        Text("Light").tag(ThemeFfi.light)
                        Text("Dark").tag(ThemeFfi.dark)
                    }
                }
                Section {
                    Toggle("Show actions on hover", isOn: binding(current, \.showHoverActions))
                    Toggle("Show key hints on the focused row", isOn: binding(current, \.showKeyHints))
                    Toggle("Show sender initials", isOn: binding(current, \.senderAvatars))
                }
            }
            .formStyle(.grouped)
        } else {
            // Canvas 3d: never a shrug. The footer already names the line, so
            // this names the way out.
            unreadable
        }
    }

    private var unreadable: some View {
        ContentUnavailableView {
            Label("This file will not parse", systemImage: "exclamationmark.triangle")
        } description: {
            Text(
                "Settings cannot be shown as fields until \(store.path) is valid TOML. "
                    + "The line below says where it went wrong; ⌘E opens it in your editor."
            )
        }
    }

    private var unbuilt: some View {
        ContentUnavailableView {
            Label("Not on macOS yet", systemImage: "gearshape")
        } description: {
            Text(
                "This section is only editable in the file for now — ⌘E opens it in your editor. "
                    + "The panes are shipping one at a time (#1156)."
            )
        }
    }

    /// The validity line. Canvas 3f puts the parse timing here, because a
    /// settings file that took a noticeable time to read is itself a finding.
    private var footer: some View {
        HStack(spacing: 8) {
            Image(systemName: store.status.valid ? "checkmark.circle" : "exclamationmark.circle")
                .foregroundStyle(store.status.valid ? Color.secondary : Color.red)
            Text(store.status.statusLine)
                .font(.system(.footnote, design: .monospaced))
                .foregroundStyle(store.status.valid ? Color.secondary : Color.red)
            if let failure = store.failure {
                Text("· \(failure)").font(.footnote).foregroundStyle(.red)
            }
            Spacer()
            Text(store.path)
                .font(.footnote)
                .foregroundStyle(.tertiary)
                .truncationMode(.head)
                .lineLimit(1)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
    }

    /// A binding that patches the file on change.
    ///
    /// There is no "save" anywhere in this window because canvas 3f decided
    /// there is no second store to save *from*.
    private func binding<T>(
        _ current: AppearanceFfi,
        _ field: WritableKeyPath<AppearanceFfi, T>
    ) -> Binding<T> {
        Binding(
            get: { current[keyPath: field] },
            set: { value in
                var next = current
                next[keyPath: field] = value
                store.apply(next)
            }
        )
    }
}
