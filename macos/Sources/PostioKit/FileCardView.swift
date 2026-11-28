import PostioFFI
import SwiftUI

/// The preview is drawn, not rendered from the file: nothing reads the file to draw a card; Space
/// hands the file itself to the system's Quick Look.
public struct FileCardView: View {
    let card: FileCardFfi?
    let focused: Bool
    @Environment(\.colorScheme) private var scheme

    public init(card: FileCardFfi?, focused: Bool) {
        self.card = card
        self.focused = focused
    }

    public enum Metrics {
        public static let height: CGFloat = 258
        public static let preview: CGFloat = 120
        public static let padding: CGFloat = 12
        public static let spacing: CGFloat = 8
        public static let radius: CGFloat = 10
        public static let tile = CGSize(width: 34, height: 42)
        public static let lines = 6
        public static let firstLine: CGFloat = 20
        public static let lineStep: CGFloat = 16
    }

    public var body: some View {
        VStack(alignment: .leading, spacing: Metrics.spacing) {
            if let card {
                preview(card)
                HStack(spacing: 10) {
                    tile(card)
                    VStack(alignment: .leading, spacing: 2) {
                        Text(SearchRuns.attributed(card.name, size: 13, weight: .semibold, scheme: scheme))
                            .lineLimit(1)
                            .truncationMode(.middle)
                        Text(card.meta)
                            .font(.system(size: 11.5))
                            .foregroundStyle(.tertiary)
                            .lineLimit(1)
                    }
                }
                Text(SearchRuns.attributed(card.line, size: 12, secondary: true, scheme: scheme))
                    .lineSpacing(3)
                    .lineLimit(2)
                    .frame(maxWidth: .infinity, minHeight: 34, alignment: .topLeading)
                Text(card.subject)
                    .font(.system(size: 11.5))
                    .foregroundStyle(.tertiary)
                    .lineLimit(1)
            } else {
                Spacer()
            }
        }
        .padding(Metrics.padding)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(
            RoundedRectangle(cornerRadius: Metrics.radius)
                .fill(
                    focused
                        ? AnyShapeStyle(Color.accentColor.opacity(SearchRuns.focusFill(scheme)))
                        : AnyShapeStyle(Color(nsColor: .windowBackgroundColor))))
        .overlay(
            RoundedRectangle(cornerRadius: Metrics.radius)
                .strokeBorder(
                    focused ? AnyShapeStyle(Color.accentColor) : AnyShapeStyle(Color(nsColor: .separatorColor)),
                    lineWidth: focused ? 2 : 1))
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(card?.accessible ?? "")
        .accessibilityAddTraits(focused ? .isSelected : [])
    }

    private func preview(_ card: FileCardFfi) -> some View {
        ZStack(alignment: .topLeading) {
            RoundedRectangle(cornerRadius: 6)
                .fill(card.preview == .image ? AnyShapeStyle(imageGround) : AnyShapeStyle(Color(nsColor: .textBackgroundColor)))
            Canvas { context, size in
                draw(card.preview, in: &context, size: size)
            }
            if let marked = card.marked {
                RoundedRectangle(cornerRadius: 2)
                    .fill(SearchRuns.highlight(scheme))
                    .frame(height: 12)
                    .padding(.horizontal, 10)
                    .offset(y: Metrics.firstLine + CGFloat(min(Int(marked), Metrics.lines - 1)) * Metrics.lineStep - 6)
            }
        }
        .frame(height: Metrics.preview)
        .clipShape(RoundedRectangle(cornerRadius: 6))
        .overlay(RoundedRectangle(cornerRadius: 6).strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1))
        .accessibilityHidden(true)
    }

    private var imageGround: LinearGradient {
        LinearGradient(
            colors: [Color.gray.opacity(0.35), Color.gray.opacity(0.18)], startPoint: .topLeading,
            endPoint: .bottomTrailing)
    }

    private func draw(_ preview: FilePreviewFfi, in context: inout GraphicsContext, size: CGSize) {
        let rule = Color(nsColor: .separatorColor)
        let ink = Color.secondary.opacity(0.28)
        switch preview {
        case .sheet:
            for row in 0...Metrics.lines {
                let y = Metrics.firstLine - Metrics.lineStep / 2 + CGFloat(row) * Metrics.lineStep
                context.fill(Path(CGRect(x: 0, y: y, width: size.width, height: 1)), with: .color(rule))
            }
            for column in 1..<5 {
                let x = size.width * CGFloat(column) / 5
                context.fill(Path(CGRect(x: x, y: 0, width: 1, height: size.height)), with: .color(rule))
            }
        case .page, .text:
            let inset: CGFloat = 14
            for line in 0..<Metrics.lines {
                let y = Metrics.firstLine + CGFloat(line) * Metrics.lineStep - 1.5
                let width = (size.width - 2 * inset) * (line % 3 == 2 ? 0.72 : 0.94)
                context.fill(
                    Path(roundedRect: CGRect(x: inset, y: y, width: width, height: 3), cornerRadius: 1.5),
                    with: .color(ink))
            }
        case .slides:
            let frame = CGRect(x: 18, y: 14, width: size.width - 36, height: size.height - 28)
            context.stroke(Path(roundedRect: frame, cornerRadius: 3), with: .color(rule))
            context.fill(
                Path(roundedRect: CGRect(x: frame.minX + 14, y: frame.minY + 12, width: frame.width * 0.5, height: 5),
                     cornerRadius: 2.5),
                with: .color(ink))
        case .image:
            break
        }
    }

    private func tile(_ card: FileCardFfi) -> some View {
        RoundedRectangle(cornerRadius: 4)
            .fill(Self.tint(card.kind))
            .overlay(RoundedRectangle(cornerRadius: 4).strokeBorder(Color(nsColor: .separatorColor), lineWidth: 1))
            .overlay(alignment: .bottom) {
                Text(card.kind)
                    .font(.system(size: 9, weight: .bold, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .minimumScaleFactor(0.7)
                    .padding(.bottom, 5)
                    .padding(.horizontal, 2)
            }
            .frame(width: Metrics.tile.width, height: Metrics.tile.height)
            .accessibilityHidden(true)
    }

    static func tint(_ kind: String) -> Color {
        switch kind {
        case "XLSX", "XLS", "CSV", "NUM", "ODS": return Color.green.opacity(0.16)
        case "PDF": return Color.red.opacity(0.14)
        case "JPG", "PNG", "GIF", "HEIC", "WEBP", "DOCX", "DOC", "PAGES": return Color.blue.opacity(0.14)
        case "PPTX", "PPT", "KEY": return Color.orange.opacity(0.16)
        default: return Color.secondary.opacity(0.1)
        }
    }
}

public struct FilesHeaderView: View {
    let header: FilesHeaderFfi

    public init(header: FilesHeaderFfi) {
        self.header = header
    }

    public var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text(header.title)
                .font(.system(size: 12, weight: .bold))
                .foregroundStyle(.secondary)
            Text(header.note)
                .font(.system(size: 12))
                .foregroundStyle(.tertiary)
                .lineLimit(1)
            Spacer(minLength: 0)
        }
        .padding(.horizontal, 20)
        .padding(.top, 14)
        .padding(.bottom, 10)
        .accessibilityElement(children: .combine)
    }
}
