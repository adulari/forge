// Shared with the MAIN APP target too (via @bacons/apple-targets' `_shared` folder convention —
// files here compile into both the widget extension and the main app), because the app's own
// LiveActivity native module (mobile/modules/live-activity) needs this same type to call
// `Activity<ForgeSessionActivityAttributes>.request/update/end`. `ContentState`'s field names
// are a hand-kept-in-sync wire contract with crates/forge-cli/src/apns.rs's
// `LiveActivityContentState` — that Rust struct's own doc comment points back here. Change one
// side, change the other, in the same commit.
import ActivityKit

struct ForgeSessionActivityAttributes: ActivityAttributes {
    struct ContentState: Codable, Hashable {
        var busy: Bool
        var waiting: Bool
        var costUsd: Double
        var contextTokens: Int

        enum CodingKeys: String, CodingKey {
            case busy, waiting
            case costUsd = "cost_usd"
            case contextTokens = "context_tokens"
        }
    }

    var sessionId: String
    var title: String
}
