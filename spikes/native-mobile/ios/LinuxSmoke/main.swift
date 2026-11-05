import Foundation

final class L: SessionListener, @unchecked Sendable {
    var n = 0
    func onUpdate(update: SessionUpdate) { n += 1; if n % 50 == 1 { print("update rev=\(update.revision) busy=\(update.busy) chars=\(update.streamingChars) blocks=\(update.streamingBlocks.count)") } }
    func onItems(items: [ChatItem]) { print("items", items.map { $0.seq }) }
    func onConnection(state: ConnectionState) { print("conn", state) }
}

let args = CommandLine.arguments
let client = DaemonClient(baseUrl: args[1])
let fleet = try await client.fleet()
print("fleet entries", fleet.count)
let h = client.session(sessionId: args[2])
let items = try await h.loadHistory(max: 1000)
print("history", items.count)
let md = parseMarkdown(text: "# hi\n\nsome **bold** text\n\n```rust\nfn x() {}\n```")
print("md blocks", md.count)
h.startStream(listener: L())
try await Task.sleep(nanoseconds: 500_000_000)
try h.sendPrompt(text: "hello from swift")
try await Task.sleep(nanoseconds: 8_000_000_000)
h.stopStream()
