import Foundation
import SwiftUI

/// Thin UI-state holder over the shared Rust core (`DaemonClient` / `SessionHandle`).
@MainActor
final class AppModel: ObservableObject {
    private let client: DaemonClient

    @Published var fleet: [FleetEntry] = []
    @Published var fleetError: String?
    @Published var items: [ChatItem] = []
    @Published var update: SessionUpdate?
    @Published var connection: ConnectionState = .connecting

    private var handle: SessionHandle?
    private var openId: String?

    init(base: String) {
        client = DaemonClient(baseUrl: base)
    }

    func refreshFleet() async {
        do {
            fleet = try await client.fleet()
            fleetError = nil
        } catch {
            fleetError = "\(error)"
        }
    }

    func open(_ id: String) async {
        guard openId != id else { return }
        close()
        openId = id
        items = []
        update = nil
        let h = client.session(sessionId: id)
        handle = h
        do {
            items = try await h.loadHistory(max: 1000)
        } catch {
            fleetError = "\(error)"
        }
        h.startStream(listener: Listener(model: self))
    }

    func send(_ text: String) {
        // Optimistic bubble (negative seq) until the daemon's own row arrives via onItems.
        items.append(ChatItem(seq: -Int64(items.count + 1), role: .user, tool: nil, blocks: parseMarkdown(text: text)))
        try? handle?.sendPrompt(text: text)
    }

    func close() {
        handle?.stopStream()
        handle = nil
        openId = nil
    }

    fileprivate func apply(update u: SessionUpdate) { update = u }

    fileprivate func apply(items fresh: [ChatItem]) {
        if var u = update {
            u.streamingBlocks = []
            u.streamingChars = 0
            update = u
        }
        items.removeAll { $0.seq < 0 }
        items.append(contentsOf: fresh)
    }

    fileprivate func apply(connection c: ConnectionState) { connection = c }
}

/// The core calls these from its tokio threads; hop to the main actor before touching UI state.
private final class Listener: SessionListener, @unchecked Sendable {
    weak var model: AppModel?
    init(model: AppModel) { self.model = model }

    func onUpdate(update: SessionUpdate) {
        Task { @MainActor in self.model?.apply(update: update) }
    }
    func onItems(items: [ChatItem]) {
        Task { @MainActor in self.model?.apply(items: items) }
    }
    func onConnection(state: ConnectionState) {
        Task { @MainActor in self.model?.apply(connection: state) }
    }
}
