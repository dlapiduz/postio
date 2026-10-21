import PostioFFI
import SwiftUI

/// The sync sheet (specs/011-focus-contacts S12, handoff screen 08): the
/// explainer with its one account row and "What syncs", or the dry run it ends
/// in. A card over a scrim; every word and key is Rust's (`SyncSheetFfi`).
public struct ContactsSyncSheetView: View {
    let sheet: SyncSheetFfi
    let model: ContactsSyncModel

    public init(sheet: SyncSheetFfi, model: ContactsSyncModel) {
        self.sheet = sheet
        self.model = model
    }

    public var body: some View {
        ZStack {
            Color.black.opacity(0.18).ignoresSafeArea()
            VStack(alignment: .leading, spacing: 14) {
                Text(sheet.title).font(.system(size: 17, weight: .bold))
                ForEach(Array(sheet.lines.enumerated()), id: \.offset) { _, line in
                    Text(line).font(.system(size: 13)).foregroundStyle(.secondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let account = sheet.account { accountRow(account) }
                if !sheet.scopes.isEmpty { scopes }
                if let plan = sheet.plan { planView(plan) }
                HStack(spacing: 8) {
                    Spacer(minLength: 0)
                    if let secondary = sheet.secondary {
                        button(secondary, primary: false) { model.done() }
                    }
                    button(sheet.primary, primary: true) { Task { await model.primary() } }
                }
                .padding(.top, 4)
            }
            .padding(24)
            .frame(width: 560, alignment: .leading)
            .background(Color(nsColor: .textBackgroundColor))
            .clipShape(RoundedRectangle(cornerRadius: 12))
            .shadow(color: .black.opacity(0.3), radius: 30, y: 18)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(sheet.title)
        }
    }

    private func accountRow(_ account: SyncAccountFfi) -> some View {
        HStack(spacing: 12) {
            Image(systemName: "person.crop.circle").font(.system(size: 20)).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 1) {
                Text(account.title).font(.system(size: 13.5, weight: .bold))
                Text(account.note).font(.system(size: 12)).foregroundStyle(.secondary)
            }
            Spacer(minLength: 0)
            Text(account.mode).font(.system(size: 12.5, weight: .semibold))
                .padding(.horizontal, 10).frame(height: 26)
                .background(RoundedRectangle(cornerRadius: 6).fill(Color.primary.opacity(0.07)))
        }
        .padding(12)
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(.separator))
    }

    private var scopes: some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("What syncs").font(.system(size: 12, weight: .bold)).foregroundStyle(.secondary)
            HStack(spacing: 2) {
                ForEach(Array(sheet.scopes.enumerated()), id: \.offset) { _, option in
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
                Spacer(minLength: 0)
            }
            .padding(2)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary))
            if let note = sheet.scopeNote {
                Text(note).font(.system(size: 11.5)).foregroundStyle(.tertiary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func planView(_ plan: SyncPlanFfi) -> some View {
        VStack(alignment: .leading, spacing: 8) {
            Text(plan.summary).font(.system(size: 14, weight: .bold))
            ScrollView {
                VStack(alignment: .leading, spacing: 5) {
                    ForEach(Array(plan.lines.enumerated()), id: \.offset) { _, line in
                        Text(line).font(.system(size: 12.5)).fixedSize(horizontal: false, vertical: true)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }
            .frame(maxHeight: 220)
            .padding(10)
            .background(RoundedRectangle(cornerRadius: 8).fill(Color(nsColor: .windowBackgroundColor)))
            Text(plan.foot).font(.system(size: 11.5)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private func button(_ button: EditButtonFfi, primary: Bool, _ action: @escaping () -> Void)
        -> some View
    {
        Button(action: action) {
            HStack(spacing: 6) {
                Text(button.label).font(.system(size: 12.5, weight: .semibold))
                if let cap = KeyCapSpelling.cap(button.key) {
                    if primary {
                        Text(cap).font(.system(size: 10, design: .monospaced))
                            .padding(.horizontal, 4).frame(minWidth: 15, minHeight: 15)
                            .background(RoundedRectangle(cornerRadius: 3).fill(.white.opacity(0.18)))
                    } else {
                        KeyCap(cap)
                    }
                }
            }
            .foregroundStyle(primary ? Color(nsColor: .textBackgroundColor) : Color.primary)
            .padding(.leading, 10).padding(.trailing, 8).frame(height: 28)
            .background(
                RoundedRectangle(cornerRadius: 6)
                    .fill(primary ? Color.primary : Color.primary.opacity(0.07)))
        }
        .buttonStyle(.plain)
    }
}
