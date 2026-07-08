// Home Screen widget: an at-a-glance list of sessions and whether each needs you. No periodic
// network refresh budget is relied on here (`.never` policy) — the app pushes a fresh timeline
// via `ExtensionStorage.reloadWidget()` whenever it has new data (foreground poll, live WS
// snapshot, or a background push waking it), which is both simpler and more battery-friendly
// than guessing a refresh interval.
import SwiftUI
import WidgetKit

struct ForgeSessionsEntry: TimelineEntry {
    let date: Date
    let sessions: [ForgeSessionSnapshot]
}

struct ForgeSessionsProvider: TimelineProvider {
    func placeholder(in context: Context) -> ForgeSessionsEntry {
        ForgeSessionsEntry(
            date: Date(),
            sessions: [
                ForgeSessionSnapshot(id: "placeholder", title: "fix the parser", busy: true, waiting: false, costUsd: 0.42)
            ]
        )
    }

    func getSnapshot(in context: Context, completion: @escaping (ForgeSessionsEntry) -> Void) {
        completion(ForgeSessionsEntry(date: Date(), sessions: ForgeSharedData.readSessions()))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<ForgeSessionsEntry>) -> Void) {
        let entry = ForgeSessionsEntry(date: Date(), sessions: ForgeSharedData.readSessions())
        completion(Timeline(entries: [entry], policy: .never))
    }
}

private struct StatusDot: View {
    let session: ForgeSessionSnapshot

    var color: Color {
        if session.waiting { return .red }
        if session.busy { return Color(red: 0.96, green: 0.46, blue: 0.10) } // ember500
        return .secondary
    }

    var body: some View {
        Circle().fill(color).frame(width: 8, height: 8)
    }
}

private struct SessionRowView: View {
    let session: ForgeSessionSnapshot

    var body: some View {
        HStack(spacing: 6) {
            StatusDot(session: session)
            Text(session.title.isEmpty ? "untitled session" : session.title)
                .font(.system(size: 13, weight: .medium))
                .lineLimit(1)
            Spacer()
            Text(String(format: "$%.2f", session.costUsd))
                .font(.system(size: 11, design: .monospaced))
                .foregroundStyle(.secondary)
        }
    }
}

struct ForgeSessionsWidgetView: View {
    var entry: ForgeSessionsProvider.Entry

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            if entry.sessions.isEmpty {
                Text("No active Forge sessions")
                    .font(.system(size: 13))
                    .foregroundStyle(.secondary)
            } else {
                ForEach(entry.sessions.prefix(4)) { session in
                    SessionRowView(session: session)
                }
            }
        }
        .padding(12)
    }
}

struct ForgeSessionsWidget: Widget {
    let kind: String = "ForgeSessionsWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: ForgeSessionsProvider()) { entry in
            ForgeSessionsWidgetView(entry: entry)
        }
        .configurationDisplayName("Forge Sessions")
        .description("See which sessions are running or waiting for you.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}
