// Shared data model for the widget extension — read-only here. The main app is the only writer
// (via the `ExtensionStorage` JS API, see mobile/src/lib/widgetData.ts), writing a small JSON
// snapshot into the App Group's UserDefaults under `sessionsKey` whenever it fetches
// `/api/sessions` or receives a live WS `Snapshot`. Field names mirror the daemon's
// `SessionRow` (crates/forge-cli/src/serve.rs) exactly — this is a hand-kept-in-sync wire
// contract across three languages (Rust -> TS -> Swift), same caveat as apns.rs's
// LiveActivityContentState.
import Foundation

let forgeAppGroup = "group.dev.adulari.forge"
let forgeSessionsKey = "sessions"

struct ForgeSessionSnapshot: Codable, Identifiable {
    var id: String
    var title: String
    var busy: Bool
    var waiting: Bool
    var costUsd: Double

    enum CodingKeys: String, CodingKey {
        case id, title, busy, waiting
        case costUsd = "cost_usd"
    }
}

enum ForgeSharedData {
    /// Every session the app last knew about, most-recently-active first (the app is
    /// responsible for ordering before it writes — the widget just renders what it's given).
    /// Returns `[]` (never throws) when there's no data yet or it's malformed, so a widget
    /// freshly installed before the app has ever synced still renders its empty state cleanly.
    static func readSessions() -> [ForgeSessionSnapshot] {
        guard let defaults = UserDefaults(suiteName: forgeAppGroup),
              let raw = defaults.string(forKey: forgeSessionsKey),
              let data = raw.data(using: .utf8)
        else {
            return []
        }
        return (try? JSONDecoder().decode([ForgeSessionSnapshot].self, from: data)) ?? []
    }
}
