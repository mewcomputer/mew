import Foundation
import MewMobileCore

struct SessionTimelineItem: Identifiable {
    let daemonNodeId: String
    let session: SessionSummary
    let activityAt: Int64

    var id: NavigationRoute { .chat(daemonNodeId: daemonNodeId, sessionId: session.sessionId) }
}

struct SessionTimelineSection: Identifiable {
    enum Period: Int, CaseIterable {
        case today, yesterday, previous7Days, previous30Days, older

        var title: String {
            switch self {
            case .today: return "Today"
            case .yesterday: return "Yesterday"
            case .previous7Days: return "Previous 7 days"
            case .previous30Days: return "Previous 30 days"
            case .older: return "Older"
            }
        }
    }

    let period: Period
    var items: [SessionTimelineItem]
    var id: Period { period }

    static func grouped(_ items: [SessionTimelineItem], now: Date, calendar: Calendar) -> [Self] {
        let sorted = items.sorted { lhs, rhs in
            if lhs.activityAt != rhs.activityAt { return lhs.activityAt > rhs.activityAt }
            if lhs.daemonNodeId != rhs.daemonNodeId { return lhs.daemonNodeId < rhs.daemonNodeId }
            return lhs.session.sessionId < rhs.session.sessionId
        }
        let today = calendar.startOfDay(for: now)
        let cutoffs: [(period: Period, timestamp: Int64)] = [
            (Period.today, 0), (.yesterday, -1), (.previous7Days, -7), (.previous30Days, -30)
        ].compactMap { period, days in
            calendar.date(byAdding: .day, value: days, to: today).map {
                (period, Int64($0.timeIntervalSince1970 * 1_000))
            }
        }
        var groups: [Period: [SessionTimelineItem]] = [:]
        for item in sorted {
            let period = item.activityAt > 0
                ? cutoffs.first(where: { item.activityAt >= $0.timestamp })?.period ?? .older
                : .older
            groups[period, default: []].append(item)
        }
        return Period.allCases.compactMap { period in
            groups[period].map { Self(period: period, items: $0) }
        }
    }
}
