import SwiftUI

private func attributed(_ spans: [MdSpan], base: Color) -> AttributedString {
    var out = AttributedString()
    for s in spans {
        var a = AttributedString(s.text)
        var font = Font.system(size: 15, weight: s.bold ? .semibold : .regular, design: s.code ? .monospaced : .default)
        if s.italic { font = font.italic() }
        a.font = font
        a.foregroundColor = s.link != nil ? T.accent : base
        if s.code { a.backgroundColor = T.bg3 }
        if let l = s.link, let url = URL(string: l) { a.link = url; a.underlineStyle = .single }
        out.append(a)
    }
    return out
}

struct MarkdownBlocks: View {
    let blocks: [MdBlock]
    var color: Color = T.ink

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(Array(blocks.enumerated()), id: \.offset) { _, b in
                BlockView(block: b, color: color)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct BlockView: View {
    let block: MdBlock
    let color: Color

    var body: some View {
        switch block {
        case .paragraph(let spans):
            Text(attributed(spans, base: color)).lineSpacing(5)
        case .heading(let level, let spans):
            Text(attributed(spans, base: T.ink))
                .font(.system(size: level == 1 ? 20 : level == 2 ? 17 : 15, weight: .semibold))
        case .listItem(let depth, let marker, let spans):
            HStack(alignment: .firstTextBaseline, spacing: 4) {
                Text(marker).foregroundColor(T.ink3)
                Text(attributed(spans, base: color)).lineSpacing(5)
            }
            .padding(.leading, CGFloat(depth) * 14)
        case .quote(let spans):
            HStack(spacing: 10) {
                Rectangle().fill(T.borderStrong).frame(width: 2)
                Text(attributed(spans, base: T.ink2)).lineSpacing(5)
            }
        case .code(let lang, let code):
            VStack(alignment: .leading, spacing: 4) {
                if !lang.isEmpty {
                    Text(lang).font(.system(size: 10, design: .monospaced)).foregroundColor(T.ink3)
                }
                ScrollView(.horizontal, showsIndicators: false) {
                    Text(code)
                        .font(.system(size: 12, design: .monospaced))
                        .foregroundColor(T.ink)
                        .fixedSize(horizontal: true, vertical: true)
                }
            }
            .padding(10)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(RoundedRectangle(cornerRadius: 8).fill(T.bg3))
        case .rule:
            Rectangle().fill(T.border).frame(height: 1)
        }
    }
}
