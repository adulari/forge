import SwiftUI

@main
struct ForgeSpikeApp: App {
    @StateObject private var model: AppModel
    @State private var openSession: String?

    init() {
        // `xcrun simctl launch ... -base http://127.0.0.1:7431/<token> -session <id>` for scripted runs.
        let d = UserDefaults.standard
        let base = d.string(forKey: "base") ?? "http://127.0.0.1:7431/a1b2c3d4e5f60718293a4b5c6d7e8f90"
        _model = StateObject(wrappedValue: AppModel(base: base))
        _openSession = State(initialValue: d.string(forKey: "session"))
    }

    var body: some Scene {
        WindowGroup {
            ZStack {
                T.bg0.ignoresSafeArea()
                if let id = openSession {
                    SessionView(model: model, id: id) { openSession = nil }
                } else {
                    FleetView(model: model) { openSession = $0 }
                }
            }
            .preferredColorScheme(.dark)
        }
    }
}
