import PostioFFI
import SwiftUI

/// The digest-this-sender sheet (specs/009-focus-macos T115, screen 24):
/// Cancel, the heading and Create along the top; the sender pre-filled (or
/// the query field, once "Match a list or a search instead…" is chosen);
/// daily, weekly or monthly with the day and the time; a preview of what
/// the rule would have caught; and the note that marked mail still comes
/// to the inbox.
///
/// Every word but "on", "at" and "Cancel" is the controller's
/// (`FocusOpenRule`/`FocusRule`); every change goes back through
/// `RuleSheetModel`, and the sheet shows what the next view says.
public struct DigestRuleSheet: View {
    let model: RuleSheetModel
    let backCap: String?
    @State private var time = ""

    public init(model: RuleSheetModel, backCap: String?) {
        self.model = model
        self.backCap = backCap
    }

    /// The sheet's width, as screen 24 draws it.
    public static let width: CGFloat = 620

    public var body: some View {
        VStack(spacing: 0) {
            if let view = model.view {
                top(view)
                Divider()
                form(view)
                    .padding(20)
            }
        }
        .frame(width: Self.width)
        .fixedSize(horizontal: false, vertical: true)
        .background(MessageSurface.content)
        .focusEffectDisabled()
    }

    private func top(_ view: RuleViewFfi) -> some View {
        ZStack {
            Text(view.heading).font(.system(size: 14, weight: .bold))
                .accessibilityAddTraits(.isHeader)
            HStack {
                FocusVerbButton("Cancel", cap: backCap, action: model.cancel)
                    .background(RoundedRectangle(cornerRadius: 7).fill(.quaternary))
                Spacer()
                FocusDefaultButton(view.create, cap: model.createCap, action: model.create)
            }
        }
        .padding(.horizontal, 10)
        .frame(height: 50)
        .background(MessageSurface.chrome)
    }

    @ViewBuilder
    private func form(_ view: RuleViewFfi) -> some View {
        VStack(alignment: .leading, spacing: 16) {
            row(view.fromLabel) {
                if let from = view.from {
                    Text(from)
                        .font(.system(size: 13, design: .monospaced))
                        .textSelection(.enabled)
                        .padding(.horizontal, 10)
                        .frame(maxWidth: .infinity, minHeight: 32, alignment: .leading)
                        .background(RoundedRectangle(cornerRadius: 6).fill(.quaternary))
                } else {
                    TextField(
                        view.placeholder,
                        text: Binding(get: { model.query }, set: { model.typed($0) })
                    )
                    .textFieldStyle(.roundedBorder)
                    .font(.system(size: 13, design: .monospaced))
                    .onSubmit(model.create)
                }
            }
            row(view.deliverLabel) { schedule(view) }
            if let heading = view.previewHeading {
                VStack(alignment: .leading, spacing: 8) {
                    Text(heading).font(.system(size: 13, weight: .semibold))
                    preview(view)
                }
            }
            Text(view.note).font(.system(size: 12.5)).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: 16) {
                if let instead = view.matchInstead {
                    Button(instead, action: model.matchInstead).buttonStyle(.link)
                }
                if let like = view.likeThis {
                    Button(like, action: model.likeThis).buttonStyle(.link)
                }
            }
            if let error = view.error {
                Text(error).font(.system(size: 12.5)).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }

    private func row<Content: View>(_ label: String, @ViewBuilder _ content: () -> Content) -> some View {
        HStack(alignment: .center, spacing: 12) {
            Text(label).font(.system(size: 13)).foregroundStyle(.secondary)
                .frame(width: 90, alignment: .leading)
            content()
        }
    }

    @ViewBuilder
    private func schedule(_ view: RuleViewFfi) -> some View {
        let schedule = view.schedule
        HStack(spacing: 8) {
            Picker("", selection: Binding(
                get: { Int(schedule.cadence) }, set: { model.setCadence($0) }
            )) {
                ForEach(Array(view.cadences.enumerated()), id: \.offset) { index, name in
                    Text(name).tag(index)
                }
            }
            .labelsHidden()
            .fixedSize()
            // Weekly names a weekday; monthly a day of the month; daily
            // neither -- the controller's cadences are in that order.
            if schedule.cadence == 1 {
                Text("on").foregroundStyle(.secondary)
                Picker("", selection: Binding(
                    get: { Int(schedule.weekday) }, set: { model.setWeekday($0) }
                )) {
                    ForEach(Array(view.weekdays.enumerated()), id: \.offset) { index, name in
                        Text(name).tag(index)
                    }
                }
                .labelsHidden()
                .fixedSize()
            } else if schedule.cadence == 2 {
                Text("on").foregroundStyle(.secondary)
                Picker("", selection: Binding(
                    get: { Int(schedule.monthDay) }, set: { model.setMonthDay($0) }
                )) {
                    ForEach(0..<28, id: \.self) { day in Text("\(day + 1)").tag(day) }
                }
                .labelsHidden()
                .fixedSize()
            }
            Text("at").foregroundStyle(.secondary)
            TextField("09:00", text: $time)
                .textFieldStyle(.roundedBorder)
                .font(.system(size: 13, design: .monospaced))
                .frame(width: 64)
                .onSubmit { model.setTime(time) }
                .onAppear { time = schedule.at }
                .onChange(of: schedule.at) { _, at in time = at }
            Spacer(minLength: 0)
        }
    }

    private func preview(_ view: RuleViewFfi) -> some View {
        VStack(spacing: 0) {
            ForEach(Array(view.preview.enumerated()), id: \.offset) { index, line in
                HStack {
                    Text(line.subject).font(.system(size: 13)).lineLimit(1)
                    Spacer()
                    Text(line.day).font(.system(size: 12).monospacedDigit()).foregroundStyle(.secondary)
                }
                .padding(.horizontal, 12)
                .frame(height: 32)
                if index < view.preview.count - 1 || view.more != nil { Divider() }
            }
            if let more = view.more {
                HStack {
                    Text(more).font(.system(size: 12.5)).foregroundStyle(.secondary)
                    Spacer()
                }
                .padding(.horizontal, 12)
                .frame(height: 30)
            }
        }
        .background(RoundedRectangle(cornerRadius: 8).fill(MessageSurface.content))
        .overlay(RoundedRectangle(cornerRadius: 8).strokeBorder(.separator))
    }
}
