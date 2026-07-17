// Hearth redesign — see mobile.dc.html's "Mobile Live Activity" (lock screen) and "Mobile
// Dynamic Island" screens (source of truth for colors/copy/layout) and HANDOFF.md's "Live
// Activity / Dynamic Island" nav-map bullet + "Design tokens" section. Allow/Deny buttons live
// in ForgeActivityIntents.swift (iOS 17+ `LiveActivityIntent`); pre-17 devices fall back to just
// the "Open" `Link`, since interactive Live Activity buttons don't exist before iOS 17.
import ActivityKit
import AppIntents
import SwiftUI
import WidgetKit

@available(iOS 16.1, *)
struct ForgeSessionActivityWidget: Widget {
    var body: some WidgetConfiguration {
        ActivityConfiguration(for: ForgeSessionActivityAttributes.self) { context in
            ForgeSessionActivityLockScreenView(attributes: context.attributes, state: context.state)
                .widgetURL(URL(string: "forge://session/\(context.attributes.sessionId)"))
                .activityBackgroundTint(ForgeActivityStyle.background)
                .activitySystemActionForegroundColor(.white)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    Image(systemName: "flame.fill")
                        .foregroundStyle(ForgeActivityStyle.accent)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    ForgeActivityBeacon(state: context.state)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    ForgeSessionActivityExpandedBody(attributes: context.attributes, state: context.state)
                }
            } compactLeading: {
                Image(systemName: "flame.fill")
                    .foregroundStyle(ForgeActivityStyle.accent)
            } compactTrailing: {
                ForgeActivityBeacon(state: context.state, size: 7)
            } minimal: {
                ForgeActivityBeacon(state: context.state, size: 9)
            }
            .widgetURL(URL(string: "forge://session/\(context.attributes.sessionId)"))
            .keylineTint(ForgeActivityStyle.color(for: context.state))
        }
    }
}

// MARK: - Hearth tokens (mirrors mobile/src/theme/tokens.ts's dark palette — see HANDOFF.md's
// "Design tokens" section; there is no way to share the TS token file with this target, so these
// are kept as the literal hex values from there).

private enum ForgeActivityStyle {
    static let background = Color(hex: 0x0E0E14)
    static let cardBg = Color(hex: 0x16161D, opacity: 0.92)
    static let cardBorder = Color(hex: 0x34343E, opacity: 0.8)
    static let trackBg = Color(hex: 0x26262E, opacity: 0.9)
    static let ink = Color(hex: 0xE9E9EF)
    static let ink2 = Color(hex: 0xA9A9B6)
    static let ink3 = Color(hex: 0x6E6E7A)
    static let accent = Color(hex: 0xFF913C)
    static let accentPressed = Color(hex: 0xF5761A)
    static let onAccent = Color(hex: 0x1B1B22)
    static let success = Color(hex: 0x7DD394)
    static let successBg = Color(hex: 0x12291A)
    static let danger = Color(hex: 0xF0716E)
    static let dangerDeep = Color(hex: 0xC24845)
    static let dangerBg = Color(hex: 0x2E1516)

    static func color(for state: ForgeSessionActivityAttributes.ContentState) -> Color {
        if state.waiting { return danger }
        if state.busy { return accent }
        return success
    }

    static func edgeGradient(for state: ForgeSessionActivityAttributes.ContentState) -> LinearGradient {
        let stops = state.waiting ? [danger, dangerDeep] : [accent, accentPressed]
        return LinearGradient(colors: stops, startPoint: .top, endPoint: .bottom)
    }

    static func glow(for state: ForgeSessionActivityAttributes.ContentState) -> Color {
        (state.waiting ? danger : accent).opacity(state.waiting ? 0.35 : 0.3)
    }

    static func ctxPercent(for state: ForgeSessionActivityAttributes.ContentState) -> Int {
        guard state.contextLimit > 0 else { return 0 }
        return Int((Double(state.contextTokens) / Double(state.contextLimit) * 100).rounded())
    }

    static func taskProgress(for state: ForgeSessionActivityAttributes.ContentState) -> Double? {
        guard let done = state.tasksDone, let total = state.tasksTotal, total > 0 else { return nil }
        return min(max(Double(done) / Double(total), 0), 1)
    }

    static func costLabel(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        String(format: "$%.2f", state.costUsd)
    }
}

private extension Color {
    init(hex: UInt32, opacity: Double = 1) {
        self.init(
            .sRGB,
            red: Double((hex >> 16) & 0xFF) / 255,
            green: Double((hex >> 8) & 0xFF) / 255,
            blue: Double(hex & 0xFF) / 255,
            opacity: opacity
        )
    }
}

// MARK: - Shared subviews

/// Forgework "emberdot pulse" (opacity 1→.35→1, busy/waiting) plus the "waiting beacon" ring
/// (scale 1→1.6 + fade, waiting only) — see HANDOFF.md's Motion section.
private struct ForgeActivityBeacon: View {
    let state: ForgeSessionActivityAttributes.ContentState
    var size: CGFloat = 8
    @State private var dotPulse = false
    @State private var ringPulse = false

    var body: some View {
        ZStack {
            if state.waiting {
                Circle()
                    .stroke(ForgeActivityStyle.danger, lineWidth: 1.5)
                    .frame(width: size, height: size)
                    .scaleEffect(ringPulse ? 1.6 : 1)
                    .opacity(ringPulse ? 0 : 1)
                    .onAppear {
                        withAnimation(.easeOut(duration: 2.8).repeatForever(autoreverses: false)) {
                            ringPulse = true
                        }
                    }
            }
            Circle()
                .fill(ForgeActivityStyle.color(for: state))
                .frame(width: size, height: size)
                .opacity(dotPulse ? 0.35 : 1)
        }
        .onAppear {
            guard state.busy || state.waiting else { return }
            withAnimation(.easeInOut(duration: state.waiting ? 0.7 : 1.0).repeatForever(autoreverses: true)) {
                dotPulse = true
            }
        }
    }
}

private struct ForgeActivityPillButton: View {
    let label: String
    let background: Color
    let foreground: Color
    var width: CGFloat?

    var body: some View {
        Text(label)
            .font(.system(size: 14, weight: .semibold))
            .frame(maxWidth: width == nil ? .infinity : nil)
            .frame(width: width, height: 40)
            .background(background)
            .foregroundStyle(foreground)
            .clipShape(RoundedRectangle(cornerRadius: 11, style: .continuous))
    }
}

/// The mono meta line on the forging card / island expanded body: `2/4 tasks · 64% ctx · $1.84`.
private struct ForgeMetaLine: View {
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 4) {
            if let done = state.tasksDone, let total = state.tasksTotal {
                Text("\(done)/\(total) tasks")
                Text("·")
            }
            Text("\(ForgeActivityStyle.ctxPercent(for: state))% ctx")
            Text("·")
            Text(ForgeActivityStyle.costLabel(for: state))
                .foregroundStyle(ForgeActivityStyle.success)
        }
        .font(.system(size: 11, design: .monospaced))
        .foregroundStyle(ForgeActivityStyle.ink3)
        .fixedSize(horizontal: true, vertical: false)
    }
}

// MARK: - Lock screen

private struct ForgeSessionActivityLockScreenView: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        Group {
            if state.waiting {
                ForgeNeedsYouCard(attributes: attributes, state: state)
            } else if state.busy {
                ForgeForgingCard(attributes: attributes, state: state)
            } else {
                ForgeIdleCard(attributes: attributes, state: state)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 8)
    }
}

private struct ForgeNeedsYouCard: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 0) {
            ForgeActivityStyle.edgeGradient(for: state)
                .frame(width: 3)
                .shadow(color: ForgeActivityStyle.glow(for: state), radius: 8, x: 2)

            VStack(alignment: .leading, spacing: 8) {
                HStack(spacing: 9) {
                    Image(systemName: "flame.fill")
                        .font(.system(size: 15))
                        .foregroundStyle(ForgeActivityStyle.accent)
                    Text(attributes.title.isEmpty ? "Forge session" : attributes.title)
                        .font(.system(size: 15.5, weight: .semibold))
                        .foregroundStyle(ForgeActivityStyle.ink)
                        .lineLimit(1)
                    Spacer(minLength: 6)
                    Text("NEEDS YOU")
                        .font(.system(size: 10, weight: .bold))
                        .tracking(0.5)
                        .foregroundStyle(ForgeActivityStyle.danger)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(ForgeActivityStyle.dangerBg)
                        .clipShape(RoundedRectangle(cornerRadius: 4, style: .continuous))
                }

                if let question = state.question, !question.isEmpty {
                    Text(question)
                        .font(.system(size: 14))
                        .foregroundStyle(ForgeActivityStyle.ink)
                        .lineLimit(3)
                }

                HStack(spacing: 6) {
                    if !attributes.agentLabel.isEmpty {
                        Text(attributes.agentLabel)
                        Text("·")
                    }
                    if let since = state.stateSinceEpoch {
                        Text("waiting") + Text(" ") + Text(Date(timeIntervalSince1970: since), style: .timer)
                    }
                }
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(ForgeActivityStyle.ink3)

                HStack(spacing: 8) {
                    if #available(iOS 17.0, *), let seq = state.promptSeq {
                        Button(intent: ForgeAllowIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                            ForgeActivityPillButton(label: "Allow", background: ForgeActivityStyle.success, foreground: ForgeActivityStyle.successBg)
                        }
                        .buttonStyle(.plain)

                        Button(intent: ForgeDenyIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                            ForgeActivityPillButton(label: "Deny", background: ForgeActivityStyle.dangerBg, foreground: ForgeActivityStyle.danger)
                        }
                        .buttonStyle(.plain)
                    }

                    if let url = URL(string: "forge://session/\(attributes.sessionId)") {
                        Link(destination: url) {
                            ForgeActivityPillButton(label: "Open", background: ForgeActivityStyle.accent, foreground: ForgeActivityStyle.onAccent, width: 92)
                        }
                    }
                }
                .padding(.top, 4)
            }
            .padding(.leading, 16)
            .padding(.trailing, 16)
            .padding(.vertical, 16)
        }
        .background(ForgeActivityStyle.cardBg)
        .overlay(
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .strokeBorder(ForgeActivityStyle.cardBorder, lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
    }
}

private struct ForgeForgingCard: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 0) {
            ForgeActivityStyle.edgeGradient(for: state)
                .frame(width: 3)
                .shadow(color: ForgeActivityStyle.glow(for: state), radius: 8, x: 2)

            VStack(alignment: .leading, spacing: 10) {
                HStack(spacing: 9) {
                    ForgeActivityBeacon(state: state, size: 8)
                    Text(attributes.title.isEmpty ? "Forge session" : attributes.title)
                        .font(.system(size: 15.5, weight: .semibold))
                        .foregroundStyle(ForgeActivityStyle.ink)
                        .lineLimit(1)
                    Spacer(minLength: 6)
                    if let since = state.stateSinceEpoch {
                        (Text("forging") + Text(" ") + Text(Date(timeIntervalSince1970: since), style: .timer))
                            .font(.system(size: 11.5, design: .monospaced))
                            .foregroundStyle(ForgeActivityStyle.ink3)
                            .fixedSize(horizontal: true, vertical: false)
                    }
                }

                HStack(spacing: 8) {
                    GeometryReader { geo in
                        ZStack(alignment: .leading) {
                            Capsule().fill(ForgeActivityStyle.trackBg)
                            Capsule()
                                .fill(LinearGradient(colors: [ForgeActivityStyle.accentPressed, ForgeActivityStyle.accent], startPoint: .leading, endPoint: .trailing))
                                .frame(width: geo.size.width * (ForgeActivityStyle.taskProgress(for: state) ?? 0))
                        }
                    }
                    .frame(height: 3)
                    ForgeMetaLine(state: state)
                }
            }
            .padding(.leading, 16)
            .padding(.trailing, 16)
            .padding(.vertical, 14)
        }
        .background(ForgeActivityStyle.cardBg)
        .overlay(
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .strokeBorder(ForgeActivityStyle.cardBorder, lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
    }
}

/// Not part of the Hearth spec screens (which only show waiting/forging) — kept for the
/// busy=false/waiting=false tail end of a session's life, styled with the same "idle rows have
/// no heat edge" rule the rest of Hearth uses (HANDOFF.md core rule 3).
private struct ForgeIdleCard: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(ForgeActivityStyle.success)
            VStack(alignment: .leading, spacing: 2) {
                Text(attributes.title.isEmpty ? "Forge session" : attributes.title)
                    .font(.system(size: 15.5, weight: .semibold))
                    .foregroundStyle(ForgeActivityStyle.ink)
                    .lineLimit(1)
                Text("Session complete")
                    .font(.system(size: 12))
                    .foregroundStyle(ForgeActivityStyle.ink3)
            }
            Spacer(minLength: 8)
            Text(ForgeActivityStyle.costLabel(for: state))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(ForgeActivityStyle.ink3)
                .fixedSize(horizontal: true, vertical: false)
        }
        .padding(16)
        .background(ForgeActivityStyle.cardBg)
        .overlay(
            RoundedRectangle(cornerRadius: 24, style: .continuous)
                .strokeBorder(ForgeActivityStyle.cardBorder, lineWidth: 1)
        )
        .clipShape(RoundedRectangle(cornerRadius: 24, style: .continuous))
    }
}

// MARK: - Dynamic Island expanded

private struct ForgeSessionActivityExpandedBody: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(attributes.title.isEmpty ? "Forge session" : attributes.title)
                .font(.system(size: 15, weight: .semibold))
                .foregroundStyle(ForgeActivityStyle.ink)
                .lineLimit(1)

            if state.waiting {
                if let question = state.question, !question.isEmpty {
                    Text(question)
                        .font(.system(size: 13.5))
                        .foregroundStyle(ForgeActivityStyle.ink2)
                        .lineLimit(2)
                }
                if #available(iOS 17.0, *), let seq = state.promptSeq {
                    HStack(spacing: 8) {
                        Button(intent: ForgeAllowIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                            ForgeActivityPillButton(label: "Allow", background: ForgeActivityStyle.success, foreground: ForgeActivityStyle.successBg)
                        }
                        .buttonStyle(.plain)

                        Button(intent: ForgeDenyIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                            ForgeActivityPillButton(label: "Deny", background: ForgeActivityStyle.dangerBg, foreground: ForgeActivityStyle.danger)
                        }
                        .buttonStyle(.plain)
                    }
                }
            } else if state.busy {
                ForgeMetaLine(state: state)
            }
        }
        .padding(.horizontal, 4)
    }
}
