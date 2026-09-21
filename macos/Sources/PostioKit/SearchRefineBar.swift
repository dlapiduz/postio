import PostioFFI
import SwiftUI

/// The strip under the toolbar while search is showing: refine chips on the
/// left, the key hints on the right (canvas 05, #1157).
///
/// A separate surface from the field on purpose, and the reason is a bug
/// that shipped: these rows were first attached to the `SearchField` itself
/// with `safeAreaInset`, and the field is hosted in an `NSToolbar` item —
/// which cannot grow downward, so the extra rows overflowed the toolbar and
/// floated over the window as a detached artifact. A toolbar item is a
/// fixed-height cell; anything below the field's row has to live where
/// layout is allowed to happen, which is here, mounted on the window's own
/// content under the toolbar.
///
/// The chips are measured, never listed: `Facets::suggested` decides which
/// narrowings are worth offering against the results actually on screen — a
/// chip that keeps none of them is a dead end, one that keeps all of them
/// appears to do nothing when clicked, and neither is offered. Four at most.
public struct SearchRefineBar: View {
    private let session: PostioSession
    /// Bumped by the engine whenever a search ran, however it ran — typed,
    /// refined, re-ordered, or picked from the sidebar. The chips are about
    /// *this* result set, so they re-measure on it.
    private let stamp: Int
    /// Narrow the current query by one token. Routed through the engine
    /// rather than run here, because the field in the toolbar has to adopt
    /// the query that actually ran — two surfaces running searches
    /// independently is two ideas of what the query is.
    private let refine: (String) -> Void

    /// Measured off the main actor — a second pass over the index that a
    /// run drawing a list must not pay for inline.
    @State private var refinements: [RefinementFfi] = []

    public init(session: PostioSession, stamp: Int, refine: @escaping (String) -> Void) {
        self.session = session
        self.stamp = stamp
        self.refine = refine
    }

    public var body: some View {
        HStack(spacing: PostioTokens.space2) {
            ForEach(refinements, id: \.token) { refinement in
                Button {
                    refine(refinement.token)
                } label: {
                    HStack(spacing: 4) {
                        Text(refinement.token)
                            .font(.system(.caption, design: .monospaced))
                        Text(Int(refinement.hits).formatted(.number))
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                    }
                    .padding(.horizontal, 6)
                    .padding(.vertical, 2)
                    .background(.quaternary.opacity(0.5), in: .rect(cornerRadius: 4))
                }
                .buttonStyle(.plain)
                .accessibilityLabel(
                    "Narrow to \(refinement.token), keeping \(refinement.hits) messages"
                )
            }
            Spacer(minLength: PostioTokens.space4)
            // `Ret open · Tab refine · ⌘S save as folder` — from the keymap,
            // like the row's own hints: this is the only place most people
            // will ever read these keys, which is what makes teaching the
            // wrong one worse than teaching none.
            let hints = session.searchHints()
            HStack(spacing: 4) {
                ForEach(Array(hints.enumerated()), id: \.offset) { index, hint in
                    if index > 0 {
                        Text("·").foregroundStyle(.tertiary)
                    }
                    Text(hint.key)
                        .font(.system(.caption2, design: .monospaced))
                    Text(hint.label)
                        .font(.caption2)
                        .foregroundStyle(.secondary)
                }
            }
            .accessibilityElement(children: .combine)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 5)
        .background(Color(nsColor: AppSurface.background))
        .overlay(alignment: .bottom) { Divider() }
        .task(id: stamp) {
            let session = session
            let measured = await Task.detached { session.refinements() }.value
            guard !Task.isCancelled else { return }
            refinements = measured
        }
    }
}
