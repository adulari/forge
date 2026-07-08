// LiveActivityModule.swift (this pod) needs the exact same ActivityAttributes type as the widget
// extension target to call Activity<ForgeSessionActivityAttributes>.request/update/end — but this
// module compiles as its own separate CocoaPod ("LiveActivity"), not merged into any target by
// @bacons/apple-targets' `_shared` convention (that only reaches the main app + extension
// targets). Kept as a literal duplicate, in lockstep, with
// mobile/targets/widget/ForgeSessionActivityAttributes.swift — change one, change both.
//
// `ContentState`'s field names are also a hand-kept-in-sync wire contract with
// crates/forge-cli/src/apns.rs's `LiveActivityContentState`.
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
