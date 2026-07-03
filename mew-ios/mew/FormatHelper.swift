import SwiftUI

/// Helpers for formatting cost, time, and session state.
enum FormatHelper {
    static func cost(_ value: Double) -> String {
        if value < 0.01 {
            return String(format: "$%.4f", value)
        }
        return String(format: "$%.2f", value)
    }

    static func relativeAge(from timestamp: UInt64?) -> String {
        guard let ts = timestamp, ts > 0 else { return "" }
        let date = Date(timeIntervalSince1970: TimeInterval(ts) / 1000)
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .abbreviated
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    static func sessionStateLabel(_ state: String) -> String {
        switch state.lowercased() {
        case "running": return "Running"
        case "active": return "Active"
        case "idle": return "Idle"
        default: return state.capitalized
        }
    }

    static func sessionStateColor(_ state: String) -> Color {
        switch state.lowercased() {
        case "running": return .blue
        case "active": return .green
        case "idle": return .gray
        default: return .gray
        }
    }

    static func sessionStateIcon(_ state: String) -> String {
        switch state.lowercased() {
        case "running": return "play.circle.fill"
        case "active": return "circle.fill"
        case "idle": return "circle"
        default: return "questionmark.circle"
        }
    }

    static func truncateNodeId(_ id: String) -> String {
        if id.count <= 16 { return id }
        return String(id.prefix(12)) + "…"
    }

    static func prettyJSON(_ jsonString: String) -> String {
        // Try to pretty-print the JSON input from permission requests
        guard let data = jsonString.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data),
              let pretty = try? JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys]),
              let prettyString = String(data: pretty, encoding: .utf8) else {
            return jsonString
        }
        return prettyString
    }
}
