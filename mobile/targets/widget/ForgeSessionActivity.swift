// Live Activity (Lock Screen + Dynamic Island) UI for one running session's turn.
// `ForgeSessionActivityAttributes` itself lives in `_shared/ForgeSessionActivityAttributes.swift`
// (shared with the main app target, which needs it too — see that file's header).
import ActivityKit
import SwiftUI
import WidgetKit

@available(iOS 16.1, *)
struct ForgeSessionActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ForgeSessionActivityAttributes.self) { context in
            ForgeSessionActivityLockScreenView(
                title: context.attributes.title,
                state: context.state
            )
            .activityBackgroundTint(Color(red: 0.09, green: 0.09, blue: 0.11)) // app backgroundColor
            .activitySystemActionForegroundColor(Color.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Text(context.attributes.title.isEmpty ? "Forge" : context.attributes.title)
                        .font(.caption)
                        .lineLimit(1)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    ForgeSessionStatusLabel(state: context.state)
                }
            } compactLeading: {
                Circle()
                    .fill(ForgeSessionStatusLabel.color(for: context.state))
                    .frame(width: 8, height: 8)
            } compactTrailing: {
                Text(String(format: "$%.2f", context.state.costUsd))
                    .font(.system(size: 11, design: .monospaced))
            } minimal: {
                Circle()
                    .fill(ForgeSessionStatusLabel.color(for: context.state))
            }
        }
    }
}

private struct ForgeSessionStatusLabel: View {
    let state: ForgeSessionActivityAttributes.ContentState

    static func color(for state: ForgeSessionActivityAttributes.ContentState) -> Color {
        if state.waiting { return .red }
        if state.busy { return Color(red: 0.96, green: 0.46, blue: 0.10) } // ember500
        return .secondary
    }

    var body: some View {
        Text(state.waiting ? "needs you" : (state.busy ? "running" : "idle"))
            .font(.caption)
            .foregroundStyle(Self.color(for: state))
    }
}

private struct ForgeSessionActivityLockScreenView: View {
    let title: String
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 4) {
                Text(title.isEmpty ? "Forge session" : title)
                    .font(.headline)
                    .foregroundStyle(.white)
                ForgeSessionStatusLabel(state: state)
            }
            Spacer()
            Text(String(format: "$%.2f", state.costUsd))
                .font(.system(size: 13, design: .monospaced))
                .foregroundStyle(.white.opacity(0.8))
        }
        .padding()
    }
}
