import PostioFFI
import SwiftUI

/// Settings > Contacts (specs/011-focus-contacts D7, handoff screen 08): the
/// explainer, "Sync with", "What syncs" and the four Suggestions switches.
/// Every word, which switch is on and what a click does is Rust's; this draws
/// them and sends the click. Sync is a dry run: the button says what a sync
/// would do and nothing is written to either side.
public struct ContactsSettingsView: View {
    let model: ContactsSettingsModel

    public init(model: ContactsSettingsModel) {
        self.model = model
    }

    public var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 18) {
                if let pane = model.pane {
                    intro(pane)
                    if let sync = pane.sync { syncWith(sync) }
                }
                if let view = model.suggestions { suggestions(view) }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .onAppear {
            model.opened()
            if DemoMode.previewsSync { Task { await model.preview() } }
        }
    }

    private func intro(_ pane: ContactsPaneFfi) -> some View {
        (Text(pane.introLead).bold() + Text(pane.introRest))
            .font(.system(size: 12.5))
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)
            .padding(14)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.6)))
    }

    private func heading(_ text: String) -> some View {
        Text(text).font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
    }

    // MARK: Sync with

    private func syncWith(_ sync: SyncSectionFfi) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            heading(sync.heading)
            VStack(spacing: 0) {
                HStack(spacing: 12) {
                    Image(systemName: "person.crop.circle")
                        .font(.system(size: 18)).foregroundStyle(.secondary)
                        .frame(width: 30, height: 30)
                        .background(Circle().fill(.quaternary))
                    VStack(alignment: .leading, spacing: 1) {
                        Text(sync.account.title).font(.system(size: 13.5, weight: .bold))
                        Text(sync.account.note).font(.system(size: 12))
                            .foregroundStyle(.secondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    Spacer(minLength: 8)
                    Menu {
                        ForEach(Array(sync.modes.enumerated()), id: \.offset) { _, option in
                            Button(option.label) { model.choose(option.mode) }
                        }
                    } label: {
                        Text(sync.account.mode).font(.system(size: 12.5))
                    }
                    .menuStyle(.button)
                    .fixedSize()
                    .accessibilityLabel("Sync mode: \(sync.account.mode)")
                }
                .padding(12)
            }
            .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.separator))
            HStack {
                Spacer(minLength: 0)
                Button {
                    Task { await model.preview() }
                } label: {
                    Text(sync.preview).font(.system(size: 12.5, weight: .semibold))
                        .padding(.horizontal, 12).frame(height: 28)
                        .foregroundStyle(Color(nsColor: .textBackgroundColor))
                        .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary))
                }
                .buttonStyle(.plain)
            }
            if let plan = sync.plan { planView(plan) }
            if let error = model.error {
                Text(error).font(.system(size: 12)).foregroundStyle(.red)
            }
            scopes(sync)
        }
    }

    private func planView(_ plan: SyncPlanFfi) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(plan.summary).font(.system(size: 13.5, weight: .bold))
            VStack(alignment: .leading, spacing: 4) {
                ForEach(Array(plan.lines.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.system(size: 12.5))
                        .fixedSize(horizontal: false, vertical: true)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.6)))
            Text(plan.foot).font(.system(size: 11.5)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let bring = plan.bringIn {
                HStack {
                    Spacer(minLength: 0)
                    Button {
                        model.bringIn()
                    } label: {
                        Text(bring.label).font(.system(size: 12.5, weight: .semibold))
                            .padding(.horizontal, 12).frame(height: 28)
                            .foregroundStyle(Color(nsColor: .textBackgroundColor))
                            .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary))
                    }
                    .buttonStyle(.plain)
                }
            }
        }
    }

    private func scopes(_ sync: SyncSectionFfi) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 14) {
                Text(sync.scopesHeading).font(.system(size: 12.5))
                HStack(spacing: 2) {
                    ForEach(Array(sync.scopes.enumerated()), id: \.offset) { _, option in
                        Button { model.choose(option.scope) } label: {
                            Text(option.label)
                                .font(.system(size: 12.5, weight: option.on ? .bold : .medium))
                                .padding(.horizontal, 11).frame(height: 26)
                                .background(
                                    RoundedRectangle(cornerRadius: 6)
                                        .fill(option.on ? Color(nsColor: .controlBackgroundColor) : .clear)
                                        .shadow(color: .black.opacity(option.on ? 0.14 : 0), radius: 1, y: 1))
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(option.on ? [.isSelected, .isButton] : .isButton)
                    }
                }
                .padding(2)
                .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
                Spacer(minLength: 0)
            }
            Text(sync.scopeNote).font(.system(size: 11.5)).foregroundStyle(.tertiary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    // MARK: Suggestions

    private func suggestions(_ view: SuggestionsViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            heading(view.heading)
            VStack(alignment: .leading, spacing: 14) {
                ForEach(Array(view.rows.enumerated()), id: \.offset) { index, row in
                    HStack(alignment: .center, spacing: 12) {
                        VStack(alignment: .leading, spacing: 2) {
                            Text(row.title).font(.system(size: 13, weight: .bold))
                            Text(row.detail).font(.system(size: 12)).foregroundStyle(.secondary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Spacer(minLength: 8)
                        Toggle(
                            row.title,
                            isOn: Binding(get: { row.on }, set: { _ in model.toggle(index) })
                        )
                        .labelsHidden()
                        .toggleStyle(.switch)
                    }
                }
            }
            if let error = view.error {
                Text(error).font(.system(size: 12)).foregroundStyle(.red)
            }
            Text(view.footer).font(.system(size: 11.5)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.top, 4)
        }
    }
}
