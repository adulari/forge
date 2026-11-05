import SwiftUI

struct SessionView: View {
    @ObservedObject var model: AppModel
    let id: String
    var onBack: () -> Void
    @State private var draft = ""

    private var statusLine: String {
        if model.connection != .open { return "connecting…" }
        guard let u = model.update else { return "idle" }
        return u.busy ? (u.activity.isEmpty ? "forging" : u.activity) : "idle"
    }

    var body: some View {
        VStack(spacing: 0) {
            HStack {
                Button(action: onBack) {
                    Text("‹").font(.system(size: 28)).foregroundColor(T.ink).padding(.horizontal, 12)
                }
                .accessibilityLabel("Back")
                VStack(alignment: .leading) {
                    Text(model.update?.title.isEmpty == false ? model.update!.title : "session")
                        .font(.system(size: 16)).foregroundColor(T.ink).lineLimit(1)
                    Text(statusLine).font(.system(size: 11, design: .monospaced)).foregroundColor(T.ink3)
                }
                Spacer()
                Circle().fill(model.update?.busy == true ? T.accent : T.ink4).frame(width: 8, height: 8)
                    .padding(.trailing, 12)
            }
            .padding(.vertical, 10)

            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        ForEach(model.items, id: \.seq) { item in
                            MessageView(item: item).id(item.seq)
                        }
                        if let live = model.update?.streamingBlocks, !live.isEmpty {
                            MarkdownBlocks(blocks: live).id("live")
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(.horizontal, 14).padding(.vertical, 8)
                }
                .defaultScrollAnchor(.bottom)
                .onChange(of: model.update?.streamingChars) { _, _ in
                    proxy.scrollTo("bottom", anchor: .bottom)
                }
                .accessibilityIdentifier("transcript")
            }

            HStack {
                TextField("Message", text: $draft, axis: .vertical)
                    .foregroundColor(T.ink)
                    .padding(.horizontal, 14).padding(.vertical, 10)
                    .background(RoundedRectangle(cornerRadius: 20).fill(T.bg3))
                    .accessibilityLabel("composer")
                Button {
                    let t = draft.trimmingCharacters(in: .whitespacesAndNewlines)
                    guard !t.isEmpty else { return }
                    model.send(t)
                    draft = ""
                } label: {
                    Text("↑").font(.system(size: 18))
                        .foregroundColor(draft.isEmpty ? T.ink3 : T.onAccent)
                        .frame(width: 40, height: 40)
                        .background(Circle().fill(draft.isEmpty ? T.bg3 : T.accent))
                }
                .accessibilityLabel("send")
            }
            .padding(10)
            .background(T.bg1)
        }
        .task { await model.open(id) }
        .onDisappear { model.close() }
    }
}

private struct MessageView: View {
    let item: ChatItem

    var body: some View {
        switch item.role {
        case .user:
            HStack {
                Spacer(minLength: 60)
                MarkdownBlocks(blocks: item.blocks)
                    .padding(.horizontal, 12).padding(.vertical, 8)
                    .background(RoundedRectangle(cornerRadius: 14).fill(T.bg3))
                    .fixedSize(horizontal: false, vertical: true)
            }
        case .assistant:
            MarkdownBlocks(blocks: item.blocks)
        default:
            Text(item.tool ?? "tool")
                .font(.system(size: 11, design: .monospaced)).foregroundColor(T.ink3)
                .padding(8).frame(maxWidth: .infinity, alignment: .leading)
                .background(RoundedRectangle(cornerRadius: 8).fill(T.bg2))
        }
    }
}
