import PostioFFI
import SwiftUI

/// The capture window (specs/009-focus-macos T116, screen 25): Cancel,
/// Task/Note and the default button along the top; where it came from;
/// the text verbatim with ⌥S for the subject; the due day with its quick
/// picks; the suggested project with its reason, ⌘P for the list; the
/// exact line it will append; and the footnote.
///
/// The words are the controller's (`FocusOpenCapture`/`FocusCapture`),
/// except "Cancel", "Task", "Note", "Due", "Change", the hint under the
/// text and the filter's placeholder, which GTK's sheet keeps as literals
/// too (`crates/postio-gtk/src/capture.rs`) and which are the same words
/// here. Every change goes back through `CaptureModel`.
public struct CaptureView: View {
    let model: CaptureModel

    @FocusState private var focus: Field?

    private enum Field: Hashable { case text, filter }

    public init(model: CaptureModel) {
        self.model = model
    }

    /// The window's width, as screen 25 draws it.
    public static let width: CGFloat = 660

    public var body: some View {
        VStack(spacing: 0) {
            if let view = model.view {
                top(view)
                Divider()
                ScrollView(.vertical) {
                    VStack(alignment: .leading, spacing: 14) {
                        Text(view.from).font(.system(size: 13)).foregroundStyle(.secondary)
                        VStack(spacing: 0) {
                            textSection(view)
                            if view.hasDue {
                                Divider()
                                dueSection(view)
                            }
                            if view.mode == .task {
                                Divider()
                                projectSection(view)
                            }
                        }
                        .background(RoundedRectangle(cornerRadius: 8).fill(MessageSurface.content))
                        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
                        previewSection(view)
                        if let error = view.error {
                            Text(error).font(.system(size: 12.5)).foregroundStyle(.red)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                    .padding(20)
                }
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(MessageSurface.chrome)
        .focusEffectDisabled()
        .onAppear { focus = .text }
        .onChange(of: model.view?.projectsOpen) { _, open in
            focus = open == true ? .filter : .text
        }
    }

    private func top(_ view: CaptureViewFfi) -> some View {
        ZStack {
            HStack(spacing: 2) {
                segment("Task", cap: model.taskCap, on: view.mode == .task) { model.choose(mode: .task) }
                segment("Note", cap: model.noteCap, on: view.mode == .note) { model.choose(mode: .note) }
            }
            .padding(2)
            .background(RoundedRectangle(cornerRadius: 7).fill(.quaternary))
            HStack {
                FocusVerbButton("Cancel", cap: model.backCap, action: model.cancel)
                    .background(RoundedRectangle(cornerRadius: 7).fill(.quaternary))
                Spacer()
                FocusDefaultButton(view.button, cap: model.buttonCap, action: model.write)
            }
        }
        .padding(.horizontal, 12)
        .frame(height: 50)
    }

    private func segment(_ title: String, cap: String?, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: { if !on { action() } }) {
            HStack(spacing: 6) {
                Text(title).font(.system(size: 13, weight: on ? .bold : .regular))
                if let cap { KeyCap(cap) }
            }
            .padding(.horizontal, 10)
            .frame(height: 26)
            .background {
                if on {
                    RoundedRectangle(cornerRadius: 6).fill(MessageSurface.content)
                        .shadow(color: .black.opacity(0.08), radius: 1, y: 0.5)
                }
            }
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .accessibilityAddTraits(on ? .isSelected : [])
    }

    private func textSection(_ view: CaptureViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 4) {
            Text(view.field).font(.system(size: 12)).foregroundStyle(.secondary)
            TextField(
                view.field, text: Binding(get: { model.text }, set: { model.typed($0) }),
                axis: .vertical
            )
            .textFieldStyle(.plain)
            .font(.system(size: 16, weight: .semibold))
            .focused($focus, equals: .text)
            if view.mode == .task {
                HStack(spacing: 5) {
                    Text("The sentence from the mail, as written \u{b7}")
                    Button(action: model.useSubject) {
                        HStack(spacing: 5) {
                            if let cap = model.subjectCap { KeyCap(cap) }
                            Text("use the subject instead")
                        }
                    }
                    .buttonStyle(.plain)
                }
                .font(.system(size: 12))
                .foregroundStyle(.secondary)
            }
        }
        .padding(12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay {
            if focus == .text {
                RoundedRectangle(cornerRadius: 8).strokeBorder(Color.accentColor, lineWidth: 2)
            }
        }
    }

    private func dueSection(_ view: CaptureViewFfi) -> some View {
        HStack(alignment: .center) {
            VStack(alignment: .leading, spacing: 2) {
                Text("Due").font(.system(size: 12)).foregroundStyle(.secondary)
                Text(view.dueLabel).font(.system(size: 14, weight: .semibold))
            }
            Spacer()
            HStack(spacing: 6) {
                ForEach(Array(view.picks.enumerated()), id: \.offset) { _, pick in
                    let on = pick.day == view.due
                    Button { model.pick(pick) } label: {
                        Text(pick.words)
                            .font(.system(size: 12.5, weight: on ? .bold : .regular))
                            .padding(.horizontal, 10)
                            .frame(height: 26)
                            .background(Capsule().fill(on ? AnyShapeStyle(MessageSurface.content) : AnyShapeStyle(.quaternary)))
                            .overlay(Capsule().strokeBorder(on ? AnyShapeStyle(.primary) : AnyShapeStyle(.clear), lineWidth: 1.5))
                            .contentShape(Capsule())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(on ? .isSelected : [])
                }
            }
        }
        .padding(12)
    }

    private func projectSection(_ view: CaptureViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 2) {
                    Text(view.projectTitle).font(.system(size: 12)).foregroundStyle(.secondary)
                    HStack(spacing: 6) {
                        Text(view.project).font(.system(size: 14, weight: .semibold))
                        Text(view.projectNote)
                            .font(.system(size: 12, design: .monospaced))
                            .foregroundStyle(.secondary)
                            .lineLimit(1)
                    }
                }
                Spacer()
                FocusVerbButton("Change", cap: model.projectCap, action: model.changeProject)
                    .background(RoundedRectangle(cornerRadius: 7).fill(MessageSurface.content))
                    .overlay(RoundedRectangle(cornerRadius: 7).strokeBorder(.separator))
            }
            if view.projectsOpen {
                VStack(spacing: 0) {
                    HStack(spacing: 6) {
                        Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                        TextField(
                            "Filter projects in your vault",
                            text: Binding(get: { model.filter }, set: { model.filtered($0) })
                        )
                        .textFieldStyle(.plain)
                        .focused($focus, equals: .filter)
                    }
                    .padding(.horizontal, 10)
                    .frame(height: 32)
                    Divider()
                    ForEach(view.projects, id: \.token) { project in
                        Button { model.choose(project.token) } label: {
                            HStack(spacing: 10) {
                                Image(systemName: "checkmark")
                                    .font(.system(size: 11, weight: .semibold))
                                    .opacity(project.chosen ? 1 : 0)
                                Text(project.name).font(.system(size: 13, weight: .semibold))
                                    .frame(width: 140, alignment: .leading)
                                Text(project.note).font(.system(size: 12, design: .monospaced))
                                    .foregroundStyle(.secondary).lineLimit(1)
                                Spacer()
                                Text(project.open).font(.system(size: 12).monospacedDigit())
                                    .foregroundStyle(.secondary)
                            }
                            .padding(.horizontal, 10)
                            .frame(height: 32)
                            .background(project.chosen ? Color.accentColor.opacity(0.08) : Color.clear)
                            .overlay {
                                if project.chosen {
                                    Rectangle().strokeBorder(Color.accentColor, lineWidth: 2)
                                }
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityAddTraits(project.chosen ? .isSelected : [])
                    }
                }
                .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.5)))
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
            }
        }
        .padding(12)
    }

    private func previewSection(_ view: CaptureViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(view.previewTitle).font(.system(size: 13, weight: .semibold))
            Text(view.preview)
                .font(.system(size: 12.5, design: .monospaced))
                .textSelection(.enabled)
                .fixedSize(horizontal: false, vertical: true)
                .padding(12)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 8).fill(MessageSurface.content))
                .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
            Text(view.footnote).font(.system(size: 12)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
