import SwiftUI

struct FleetView: View {
    @ObservedObject var model: AppModel
    var onOpen: (String) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Text("Fleet")
                .font(.system(size: 22, weight: .semibold))
                .foregroundColor(T.ink)
                .padding(.horizontal, 16).padding(.top, 12).padding(.bottom, 8)
            if let err = model.fleetError {
                Text(err).font(.system(size: 12)).foregroundColor(T.danger).padding(16)
            }
            ScrollView {
                LazyVStack(spacing: 8) {
                    ForEach(model.fleet, id: \.stableKey) { entry in
                        switch entry {
                        case .label(let text):
                            Text(text)
                                .font(.system(size: 11, design: .monospaced)).tracking(1.2)
                                .foregroundColor(T.ink3)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .padding(.top, 10).padding(.leading, 4)
                        case .session(let row):
                            SessionCard(row: row).onTapGesture { onOpen(row.id) }
                        }
                    }
                }
                .padding(.horizontal, 12)
            }
        }
        .task {
            while !Task.isCancelled {
                await model.refreshFleet()
                try? await Task.sleep(nanoseconds: 5_000_000_000)
            }
        }
    }
}

private extension FleetEntry {
    var stableKey: String {
        switch self {
        case .label(let text): return "l:" + text
        case .session(let row): return row.id
        }
    }
}

private struct SessionCard: View {
    let row: SessionRow

    var state: String { row.waiting ? "waiting" : row.busy ? "busy" : "idle" }
    var dot: Color { state == "waiting" ? T.danger : state == "busy" ? T.accent : T.ink4 }
    var project: String { row.cwd.split(separator: "/").last.map(String.init) ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: 5) {
            HStack(spacing: 8) {
                Circle().fill(dot).frame(width: state == "idle" ? 7 : 8, height: state == "idle" ? 7 : 8)
                Text(row.title.isEmpty ? "session \(row.id.prefix(8))" : row.title)
                    .font(.system(size: state == "idle" ? 13.5 : 14.5))
                    .foregroundColor(state == "idle" ? T.ink2 : T.ink)
                    .lineLimit(1)
                Spacer()
                if row.lastTurnOutcome == "failed" {
                    Text("failed").font(.system(size: 10)).foregroundColor(T.danger)
                }
                Text(state == "waiting" ? "now" : state == "busy" ? "running" : String(format: "$%.2f", row.costUsd))
                    .font(.system(size: 11, design: .monospaced))
                    .foregroundColor(state == "waiting" ? T.danger : T.ink3)
            }
            Text(row.waiting ? "needs a decision" : "local · " + (row.model.components(separatedBy: "::").last ?? row.model))
                .font(.system(size: 11, design: .monospaced))
                .foregroundColor(state == "waiting" ? T.ink2 : T.ink4)
                .lineLimit(1)
            if !row.waiting {
                HStack(spacing: 12) {
                    Text(project).foregroundColor(T.ink3)
                    if let b = row.branch { Text("⑂ \(b)").foregroundColor(T.ink3).lineLimit(1) }
                    if let a = row.diffAdded { Text("+\(a) −\(row.diffRemoved ?? 0)").foregroundColor(T.success) }
                }
                .font(.system(size: 11, design: .monospaced))
            }
        }
        .padding(.horizontal, 14).padding(.vertical, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: 12).fill(row.waiting ? T.selection : T.bg2))
        .overlay(RoundedRectangle(cornerRadius: 12).stroke(T.border, lineWidth: 1))
        .contentShape(Rectangle())
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("\(row.title), \(state)")
        .accessibilityAddTraits(.isButton)
    }
}
