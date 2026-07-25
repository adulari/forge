// Machined redesign — docs/design/machined/"Forge Machined - Mobile.dc.html" L610-627 ("M Native
// Surfaces") is the source of truth for colors/copy/layout; INVENTORY.md §05 "Native iOS" is the
// frame map. This supersedes the retired Emberline/Hearth styling: the thermal heat edge, the
// ember glows and the gradient progress fill are gone — flat fills, hairline borders, one ember
// accent. Palette + typeface rationale live in ForgeMachinedStyle.swift.
//
// Three presentations, each straight off the design frame:
//   · lock screen  — permission card (spark + title + NEEDS YOU/timer, question, Allow/Deny/Open
//                    outlined at 7pt) when waiting; a one-glance forging card otherwise.
//   · island compact — spark leading + mono pace text trailing (`2/4 · 64%`).
//   · island expanded — spark, elapsed timer, then the permission mini-card with pill Allow/Deny.
//
// Allow/Deny are backed by ForgeActivityIntents.swift (iOS 17+ `LiveActivityIntent`); pre-17
// devices fall back to just an "Open" `Link`, since interactive Live Activity buttons don't exist
// before iOS 17. That interaction contract is unchanged by this restyle.
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
                // The system container is the card — tinting it (rather than drawing a second
                // rounded rect inside it) is what keeps this a single Machined surface instead of
                // a card-inside-a-card with mismatched corner radii.
                .activityBackgroundTint(ForgeMachined.panel)
                .activitySystemActionForegroundColor(ForgeMachined.accent)
        } dynamicIsland: { context in
            DynamicIsland {
                DynamicIslandExpandedRegion(.leading) {
                    ForgeSparkMark(size: 12)
                }
                DynamicIslandExpandedRegion(.trailing) {
                    ForgeElapsedLabel(state: context.state)
                }
                DynamicIslandExpandedRegion(.bottom) {
                    ForgeSessionActivityExpandedBody(attributes: context.attributes, state: context.state)
                }
            } compactLeading: {
                ForgeSparkMark(size: 11)
            } compactTrailing: {
                ForgePaceLabel(state: context.state)
            } minimal: {
                ForgeStatusDot(color: ForgeActivityState.tint(for: context.state), size: 7)
            }
            .widgetURL(URL(string: "forge://session/\(context.attributes.sessionId)"))
            .keylineTint(ForgeActivityState.tint(for: context.state))
        }
    }
}

// MARK: - Derived state

private enum ForgeActivityState {
    static func tint(for state: ForgeSessionActivityAttributes.ContentState) -> Color {
        if state.waiting { return ForgeMachined.danger }
        if state.busy { return ForgeMachined.accent }
        return ForgeMachined.success
    }

    static func title(_ attributes: ForgeSessionActivityAttributes) -> String {
        attributes.title.isEmpty ? "Forge session" : attributes.title
    }

    static func ctxPercent(for state: ForgeSessionActivityAttributes.ContentState) -> Int {
        guard state.contextLimit > 0 else { return 0 }
        return Int((Double(state.contextTokens) / Double(state.contextLimit) * 100).rounded())
    }

    static func costLabel(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        String(format: "$%.2f", state.costUsd)
    }

    /// Compact Dynamic Island pace text — the design's `2/4 · 64%`, degrading to just the context
    /// percentage when the session has no task list.
    static func paceLabel(for state: ForgeSessionActivityAttributes.ContentState) -> String {
        let ctx = "\(ctxPercent(for: state))%"
        guard let done = state.tasksDone, let total = state.tasksTotal, total > 0 else { return ctx }
        return "\(done)/\(total) · \(ctx)"
    }
}

// MARK: - Shared subviews

/// Dynamic Island compact trailing: the design's mono pace text, tinted by state so a waiting
/// session still reads as urgent at a glance in the smallest presentation.
private struct ForgePaceLabel: View {
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        Text(ForgeActivityState.paceLabel(for: state))
            .font(ForgeMachined.mono(10))
            .foregroundStyle(ForgeActivityState.tint(for: state))
            .lineLimit(1)
            .minimumScaleFactor(0.8)
    }
}

/// Dynamic Island expanded trailing: elapsed time in the current state (the design's `12s`).
private struct ForgeElapsedLabel: View {
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        Group {
            if let since = state.stateSinceEpoch {
                Text(Date(timeIntervalSince1970: since), style: .timer)
            } else {
                Text(state.waiting ? "waiting" : "forging")
            }
        }
        .font(ForgeMachined.mono(10.5))
        .foregroundStyle(state.waiting ? ForgeMachined.danger : ForgeMachined.ink3)
        .lineLimit(1)
        .multilineTextAlignment(.trailing)
    }
}

/// The mono meta line: `forging 4m · 2/4 tasks · 64% ctx`.
private struct ForgeMetaLine: View {
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 5) {
            if let since = state.stateSinceEpoch {
                (Text("forging ") + Text(Date(timeIntervalSince1970: since), style: .timer))
                Text("·")
            }
            if let done = state.tasksDone, let total = state.tasksTotal, total > 0 {
                Text("\(done)/\(total) tasks")
                Text("·")
            }
            Text("\(ForgeActivityState.ctxPercent(for: state))% ctx")
            Spacer(minLength: 0)
        }
        .font(ForgeMachined.mono(10.5))
        .foregroundStyle(ForgeMachined.ink3)
        .lineLimit(1)
    }
}

/// Lock-screen action: an outlined 7pt-radius button. Machined does not fill action surfaces —
/// emphasis is carried by border strength and ink, not by a colored block.
private struct ForgeOutlineButton: View {
    enum Emphasis { case primary, quiet }

    let label: String
    let emphasis: Emphasis

    var body: some View {
        Text(label)
            .font(ForgeMachined.sans(12.5, weight: emphasis == .primary ? .semibold : .regular))
            .foregroundStyle(emphasis == .primary ? ForgeMachined.ink : ForgeMachined.ink2)
            .frame(maxWidth: .infinity)
            .frame(height: 38)
            .contentShape(Rectangle())
            .overlay(
                RoundedRectangle(cornerRadius: 7, style: .continuous)
                    .strokeBorder(
                        emphasis == .primary ? ForgeMachined.borderStrong : ForgeMachined.borderSoft,
                        lineWidth: 1
                    )
            )
    }
}

/// Dynamic Island expanded action: the design uses tinted pills here rather than the lock
/// screen's outlines, because the island has no card edge of its own to sit against.
private struct ForgePillButton: View {
    let label: String
    let tint: Color
    let fill: Color

    var body: some View {
        Text(label)
            .font(ForgeMachined.sans(11.5, weight: .semibold))
            .foregroundStyle(tint)
            .frame(maxWidth: .infinity)
            .frame(height: 30)
            .background(fill, in: Capsule())
            .contentShape(Capsule())
    }
}

// MARK: - Lock screen

private struct ForgeSessionActivityLockScreenView: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        Group {
            if state.waiting {
                ForgePermissionCard(attributes: attributes, state: state)
            } else if state.busy {
                ForgeForgingCard(attributes: attributes, state: state)
            } else {
                ForgeIdleCard(attributes: attributes, state: state)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }
}

private struct ForgePermissionCard: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 10) {
                ForgeSparkMark(size: 12)
                Text(ForgeActivityState.title(attributes))
                    .font(ForgeMachined.sans(13.5, weight: .semibold))
                    .foregroundStyle(ForgeMachined.ink)
                    .lineLimit(1)
                Spacer(minLength: 6)
                needsYouLabel
            }

            if let question = state.question, !question.isEmpty {
                Text(question)
                    .font(ForgeMachined.sans(12.5))
                    .foregroundStyle(ForgeMachined.inkBody)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: 9) {
                if #available(iOS 17.0, *), let seq = state.promptSeq {
                    Button(intent: ForgeAllowIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                        ForgeOutlineButton(label: "Allow", emphasis: .primary)
                    }
                    .buttonStyle(.plain)

                    Button(intent: ForgeDenyIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                        ForgeOutlineButton(label: "Deny", emphasis: .quiet)
                    }
                    .buttonStyle(.plain)
                }

                // Always present — and the only action on pre-iOS-17 devices, where it stretches
                // to the full width because the two intent buttons above don't exist.
                if let url = URL(string: "forge://session/\(attributes.sessionId)") {
                    Link(destination: url) {
                        ForgeOutlineButton(label: "Open", emphasis: .quiet)
                    }
                }
            }
        }
    }

    private var needsYouLabel: some View {
        Group {
            if let since = state.stateSinceEpoch {
                Text("NEEDS YOU · ") + Text(Date(timeIntervalSince1970: since), style: .timer)
            } else {
                Text("NEEDS YOU")
            }
        }
        .font(ForgeMachined.mono(10.5, weight: .medium))
        .foregroundStyle(ForgeMachined.danger)
        .lineLimit(1)
    }
}

/// The design's second lock-screen card: one glance line plus the mono meta line. No progress
/// bar — Machined carries pace in the numbers, not in a gradient track.
private struct ForgeForgingCard: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 7) {
            HStack(spacing: 9) {
                ForgeStatusDot(color: ForgeMachined.accent, size: 5)
                Text(ForgeActivityState.title(attributes))
                    .font(ForgeMachined.sans(13.5, weight: .semibold))
                    .foregroundStyle(ForgeMachined.ink)
                    .lineLimit(1)
                Spacer(minLength: 6)
                Text(ForgeActivityState.costLabel(for: state))
                    .font(ForgeMachined.mono(10.5))
                    .foregroundStyle(ForgeMachined.ink3)
                    .lineLimit(1)
            }
            ForgeMetaLine(state: state)
        }
    }
}

/// Not a design frame (the spec only draws waiting/forging) — this covers the
/// busy=false/waiting=false tail end of a session's life, in the same flat Machined language.
private struct ForgeIdleCard: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        HStack(spacing: 9) {
            ForgeStatusDot(color: ForgeMachined.success, size: 5)
            VStack(alignment: .leading, spacing: 2) {
                Text(ForgeActivityState.title(attributes))
                    .font(ForgeMachined.sans(13.5, weight: .semibold))
                    .foregroundStyle(ForgeMachined.ink)
                    .lineLimit(1)
                Text("Session complete")
                    .font(ForgeMachined.sans(11.5))
                    .foregroundStyle(ForgeMachined.ink2)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            Text(ForgeActivityState.costLabel(for: state))
                .font(ForgeMachined.mono(10.5))
                .foregroundStyle(ForgeMachined.ink3)
                .lineLimit(1)
        }
    }
}

// MARK: - Dynamic Island expanded

private struct ForgeSessionActivityExpandedBody: View {
    let attributes: ForgeSessionActivityAttributes
    let state: ForgeSessionActivityAttributes.ContentState

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(ForgeActivityState.title(attributes))
                .font(ForgeMachined.sans(12.5, weight: .semibold))
                .foregroundStyle(ForgeMachined.ink)
                .lineLimit(1)

            if state.waiting {
                if let question = state.question, !question.isEmpty {
                    Text(question)
                        .font(ForgeMachined.sans(11.5))
                        .foregroundStyle(ForgeMachined.ink2)
                        .lineLimit(2)
                        .fixedSize(horizontal: false, vertical: true)
                }

                if #available(iOS 17.0, *), let seq = state.promptSeq {
                    HStack(spacing: 9) {
                        Button(intent: ForgeAllowIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                            ForgePillButton(
                                label: "Allow",
                                tint: ForgeMachined.success,
                                fill: ForgeMachined.success.opacity(0.18)
                            )
                        }
                        .buttonStyle(.plain)

                        Button(intent: ForgeDenyIntent(sessionId: attributes.sessionId, baseUrl: attributes.baseUrl, seq: seq)) {
                            ForgePillButton(
                                label: "Deny",
                                tint: ForgeMachined.danger,
                                fill: ForgeMachined.danger.opacity(0.15)
                            )
                        }
                        .buttonStyle(.plain)
                    }
                } else if let url = URL(string: "forge://session/\(attributes.sessionId)") {
                    // Pre-iOS 17, or a waiting state that arrived without a prompt sequence to
                    // answer: the decision can only be made in the app.
                    Link(destination: url) {
                        ForgePillButton(
                            label: "Open in Forge",
                            tint: ForgeMachined.accent,
                            fill: ForgeMachined.accent.opacity(0.16)
                        )
                    }
                }
            } else if state.busy {
                ForgeMetaLine(state: state)
            }
        }
        .padding(.horizontal, 4)
        .padding(.bottom, 2)
    }
}
