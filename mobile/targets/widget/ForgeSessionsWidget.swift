// Home Screen widget — the design's 170×170 "fleet glance" (docs/design/machined/"Forge Machined
// - Mobile.dc.html" L610-627, INVENTORY.md §05): spark + "Fleet" + today's spend in the header,
// up to three session rows each with a status dot, and a "N needs you" footer pinned to the
// bottom. Machined styling; palette + typeface rationale in ForgeMachinedStyle.swift.
//
// No periodic network refresh budget is relied on here (`.never` policy) — the app pushes a fresh
// timeline via `ExtensionStorage.reloadWidget()` whenever it has new data (foreground poll, live
// WS snapshot, or a background push waking it), which is both simpler and more battery-friendly
// than guessing a refresh interval.
//
// Every row an installed widget renders comes from `ForgeSharedData` (the App Group the app
// writes, see mobile/src/lib/widgetData.ts). The sample rows below are reachable ONLY from
// `placeholder(in:)` and from `getSnapshot` when `context.isPreview` — i.e. the widget gallery
// and SwiftUI previews. A real widget with no synced data renders the empty state, never fiction.
import SwiftUI
import WidgetKit

struct ForgeSessionsEntry: TimelineEntry {
    let date: Date
    let sessions: [ForgeSessionSnapshot]
}

/// Gallery + SwiftUI-preview rows only. Mirrors the three sessions drawn in the design frame.
enum ForgeWidgetSampleData {
    static let sessions: [ForgeSessionSnapshot] = [
        ForgeSessionSnapshot(id: "sample-1", title: "vol-mom sweep", busy: false, waiting: true, costUsd: 1.84),
        ForgeSessionSnapshot(id: "sample-2", title: "mesh failover", busy: true, waiting: false, costUsd: 0.31),
        ForgeSessionSnapshot(id: "sample-3", title: "catalog loader", busy: false, waiting: false, costUsd: 0.12),
    ]
}

struct ForgeSessionsProvider: TimelineProvider {
    func placeholder(in context: Context) -> ForgeSessionsEntry {
        ForgeSessionsEntry(date: Date(), sessions: ForgeWidgetSampleData.sessions)
    }

    func getSnapshot(in context: Context, completion: @escaping (ForgeSessionsEntry) -> Void) {
        let sessions = ForgeSharedData.readSessions()
        // The widget gallery asks for a snapshot before the widget has ever been installed, so
        // there is usually nothing in the App Group yet. Sample rows are shown there — and only
        // there — so the gallery tile isn't an empty box.
        if sessions.isEmpty, context.isPreview {
            completion(ForgeSessionsEntry(date: Date(), sessions: ForgeWidgetSampleData.sessions))
            return
        }
        completion(ForgeSessionsEntry(date: Date(), sessions: sessions))
    }

    func getTimeline(in context: Context, completion: @escaping (Timeline<ForgeSessionsEntry>) -> Void) {
        let entry = ForgeSessionsEntry(date: Date(), sessions: ForgeSharedData.readSessions())
        completion(Timeline(entries: [entry], policy: .never))
    }
}

private struct ForgeFleetRow: View {
    let session: ForgeSessionSnapshot

    private var dotColor: Color {
        if session.waiting { return ForgeMachined.danger }
        if session.busy { return ForgeMachined.accent }
        return ForgeMachined.success
    }

    var body: some View {
        HStack(spacing: 8) {
            ForgeStatusDot(color: dotColor, size: 4)
            // The row that needs you is the only one drawn at body ink; the rest recede.
            Text(session.title.isEmpty ? "untitled session" : session.title)
                .font(.system(size: 10.5))
                .foregroundStyle(session.waiting ? ForgeMachined.inkBody : ForgeMachined.ink2)
                .lineLimit(1)
                .truncationMode(.tail)
            Spacer(minLength: 0)
        }
    }
}

struct ForgeSessionsWidgetView: View {
    @Environment(\.widgetFamily) private var family

    var entry: ForgeSessionsProvider.Entry

    // Three rows is the design's small tile; medium gets the full four the app ever syncs
    // (`MAX_SESSIONS` in mobile/src/lib/widgetData.ts).
    private var rowLimit: Int { family == .systemSmall ? 3 : 4 }
    private var needsYou: Int { entry.sessions.filter { $0.waiting }.count }
    private var forging: Int { entry.sessions.filter { $0.busy && !$0.waiting }.count }
    private var totalCost: Double { entry.sessions.reduce(0) { $0 + $1.costUsd } }

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            HStack(spacing: 8) {
                ForgeSparkMark(size: 11)
                Text("Fleet")
                    .font(.system(size: 11, weight: .semibold))
                    .foregroundStyle(ForgeMachined.ink)
                    .lineLimit(1)
                Spacer(minLength: 4)
                if !entry.sessions.isEmpty {
                    Text(String(format: "$%.2f", totalCost))
                        .font(ForgeMachined.mono(10))
                        .foregroundStyle(ForgeMachined.ink3)
                        .lineLimit(1)
                }
            }

            if entry.sessions.isEmpty {
                Text("No sessions yet")
                    .font(.system(size: 11))
                    .foregroundStyle(ForgeMachined.ink2)
                    .lineLimit(1)
                Text("Open Forge to sync your fleet.")
                    .font(ForgeMachined.mono(9.5))
                    .foregroundStyle(ForgeMachined.ink3)
                    .lineLimit(2)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                VStack(alignment: .leading, spacing: 9) {
                    ForEach(entry.sessions.prefix(rowLimit)) { session in
                        ForgeFleetRow(session: session)
                    }
                }
            }

            Spacer(minLength: 0)

            Text(footerText)
                .font(ForgeMachined.mono(10))
                .foregroundStyle(needsYou > 0 ? ForgeMachined.danger : ForgeMachined.ink3)
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .forgeWidgetSurface()
    }

    /// Honest about what the widget actually knows: it never claims a fleet state it hasn't been
    /// given (the "not synced" case is the empty App Group, not "you have zero sessions").
    private var footerText: String {
        if needsYou > 0 { return needsYou == 1 ? "1 needs you" : "\(needsYou) need you" }
        if entry.sessions.isEmpty { return "not synced" }
        if forging > 0 { return forging == 1 ? "1 forging" : "\(forging) forging" }
        return "all idle"
    }
}

private extension View {
    /// iOS 17 widgets must declare their background through `containerBackground(_:for:)`, which
    /// also supplies the tile's content margins — so the design's 13pt inset is only applied by
    /// hand on iOS 16.1-16.x, which has neither.
    @ViewBuilder
    func forgeWidgetSurface() -> some View {
        if #available(iOS 17.0, *) {
            containerBackground(ForgeMachined.panel, for: .widget)
        } else {
            padding(13).background(ForgeMachined.panel)
        }
    }
}

struct ForgeSessionsWidget: Widget {
    // Widget identity — changing this orphans every already-installed tile, so it stays as-is
    // through the Machined restyle.
    let kind: String = "ForgeSessionsWidget"

    var body: some WidgetConfiguration {
        StaticConfiguration(kind: kind, provider: ForgeSessionsProvider()) { entry in
            ForgeSessionsWidgetView(entry: entry)
        }
        .configurationDisplayName("Forge Fleet")
        .description("Which sessions are running, and which need you.")
        .supportedFamilies([.systemSmall, .systemMedium])
    }
}
