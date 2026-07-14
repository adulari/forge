import ActivityKit
import SwiftUI
import WidgetKit

@available(iOS 16.1, *)
struct ForgeSessionActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ForgeSessionActivityAttributes.self) { context in
            ForgeSessionActivityLockScreenView(title: context.attributes.title, state: context.state)
                .widgetURL(URL(string: "forge://session/\(context.attributes.sessionId)"))
                .activityBackgroundTint(ForgeActivityStyle.background)
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    ForgeActivityStatus(state: context.state, showsLabel: true)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    Text(context.attributes.title.isEmpty ? "Forge" : context.attributes.title)
                        .font(.caption.weight(.semibold))
                        .lineLimit(1)
                        .multilineTextAlignment(.trailing)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    VStack(alignment: .leading, spacing: 10) {
                        HStack(spacing: 8) {
                            Image(systemName: ForgeActivityStyle.symbol(for: context.state))
                                .foregroundStyle(ForgeActivityStyle.color(for: context.state))
                            Text(ForgeActivityStyle.detail(for: context.state))
                                .font(.subheadline.weight(.medium))
                            Spacer(minLength: 8)
                            Text(ForgeActivityStyle.contextLabel(for: context.state))
                                .font(.caption.monospacedDigit())
                                .foregroundStyle(.secondary)
                        }
                        ProgressView(value: ForgeActivityStyle.contextProgress(for: context.state))
                            .tint(ForgeActivityStyle.color(for: context.state))
                        HStack {
                            Label(ForgeActivityStyle.costLabel(for: context.state), systemImage: "sparkles")
                            Spacer()
                            Text("Live session")
                        }
                        .font(.caption)
                        .foregroundStyle(.secondary)
                    }
                }
            } compactLeading: {
                HStack(spacing: 4) {
                    Circle().fill(ForgeActivityStyle.color(for: context.state)).frame(width: 7, height: 7)
                    Image(systemName: ForgeActivityStyle.compactSymbol(for: context.state))
                        .font(.caption2.weight(.bold))
                }
            } compactTrailing: {
                Text(ForgeActivityStyle.compactContext(for: context.state))
                    .font(.caption2.monospacedDigit().weight(.semibold))
                    .foregroundStyle(ForgeActivityStyle.color(for: context.state))
            } minimal: {
                Image(systemName: ForgeActivityStyle.compactSymbol(for: context.state))
                    .font(.caption.weight(.bold))
                    .foregroundStyle(ForgeActivityStyle.color(for: context.state))
            }
            .widgetURL(URL(string: "forge://session/\(context.attributes.sessionId)"))
            .keylineTint(ForgeActivityStyle.color(for: context.state))
        }
    }
}

private enum ForgeActivityStyle {
    static let background = Color(red: 0.09, green: 0.09, blue: 0.11)
    static let ember = Color(red: 0.96, green: 0.46, blue: 0.10)
    static let waiting = Color(red: 1.0, green: 0.71, blue: 0.20)

    static func color(for state: ForgeSessionActivityAttributes.ContentState) -> Color {
        if state.waiting { return waiting }
        if state.busy { return ember }
        return .green
    }

    static func symbol(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        if state.waiting { return "hand.raised.fill" }
        if state.busy { return "bolt.fill" }
        return "checkmark.circle.fill"
    }

    static func compactSymbol(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        state.waiting ? "exclamationmark" : (state.busy ? "bolt.fill" : "checkmark")
    }

    static func label(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        state.waiting ? "Waiting for you" : (state.busy ? "Running" : "Complete")
    }

    static func detail(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        state.waiting ? "Forge needs your input" : (state.busy ? "Working on your session" : "Session complete")
    }

    static func contextProgress(for state: ForgeSessionActivityAttributes.ContentState) -> Double {
        min(max(Double(state.contextTokens) / 200_000, 0), 1)
    }

    static func contextLabel(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        "\(max(state.contextTokens, 0) / 1000)k context"
    }

    static func compactContext(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        state.waiting ? "INPUT" : "\(max(state.contextTokens, 0) / 1000)k"
    }

    static func costLabel(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        String(format: "$%.2f used", state.costUsd)
    }
}

private struct ForgeActivityStatus: View {
    let state: ForgeSessionActivityAttributes.ContentState
    let showsLabel: Bool

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: ForgeActivityStyle.symbol(for: state))
            if showsLabel { Text(ForgeActivityStyle.label(for: state)) }
        }
        .font(.caption.weight(.semibold))
        .foregroundStyle(ForgeActivityStyle.color(for: state))
    }
}

private struct ForgeSessionActivityLockScreenView: View {
    let title: String
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack(alignment: .top, spacing: 10) {
                ZStack {
                    Circle().fill(ForgeActivityStyle.color(for: state).opacity(0.18))
                    Image(systemName: ForgeActivityStyle.symbol(for: state))
                        .font(.headline)
                        .foregroundStyle(ForgeActivityStyle.color(for: state))
                }
                .frame(width: 34, height: 34)
                VStack(alignment: .leading, spacing: 3) {
                    Text(title.isEmpty ? "Forge session" : title)
                        .font(.headline.weight(.semibold))
                        .lineLimit(1)
                    ForgeActivityStatus(state: state, showsLabel: true)
                }
                Spacer(minLength: 8)
                Text(ForgeActivityStyle.costLabel(for: state))
                    .font(.caption.monospacedDigit())
                    .foregroundStyle(.white.opacity(0.68))
            }
            VStack(alignment: .leading, spacing: 6) {
                HStack {
                    Text(state.waiting ? "Action required" : "Context window")
                        .font(.caption.weight(.medium))
                    Spacer()
                    Text(ForgeActivityStyle.contextLabel(for: state))
                        .font(.caption.monospacedDigit())
                }
                .foregroundStyle(.white.opacity(0.72))
                ProgressView(value: ForgeActivityStyle.contextProgress(for: state))
                    .tint(ForgeActivityStyle.color(for: state))
            }
        }
        .padding(.vertical, 2)
        .foregroundStyle(.white)
    }
}
