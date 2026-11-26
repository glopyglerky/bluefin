import Foundation

/// Event state comes from the schedule, never inferred from a guide's ON_NOW flag.
enum SportsGameStatus: String, CaseIterable, Identifiable {
    case live, upcoming, final, delayed, postponed, cancelled, unknown
    var id: String { rawValue }
    var label: String {
        switch self {
        case .live: return "Live"
        case .upcoming: return "Upcoming"
        case .final: return "Final"
        case .delayed: return "Delayed"
        case .postponed: return "Postponed"
        case .cancelled: return "Cancelled"
        case .unknown: return "Status unavailable"
        }
    }
}

extension SportsOption {
    var status: SportsGameStatus {
        // UNKNOWN remains unknown even if an embedded event contains a stale state.
        guard kind != "UNKNOWN", kind != "ON_NOW" else { return .unknown }
        switch event?.state?.uppercased() {
        case "POST", "FINAL", "COMPLETED", "FINISHED": return .final
        case "IN", "LIVE", "IN_PROGRESS": return .live
        case "PRE", "SCHEDULED", "UPCOMING": return .upcoming
        case "POSTPONED": return .postponed
        case "DELAYED": return .delayed
        case "CANCELED", "CANCELLED": return .cancelled
        default:
            if kind == "FINAL" { return .final }
            if kind == "LIVE" { return .live }
            if kind == "UPCOMING" { return .upcoming }
            return .unknown
        }
    }
    var scheduleDate: Date? { [event?.date, start, native_start].compactMap { SportsSchedule.date($0) }.first }
    var showsScores: Bool { status == .live || status == .final }
    var broadcastLabel: String {
        if kind == "ON_NOW" { return "On now · Guide listing" }
        if kind == "RECORDING" { return "Recording" }
        return status.label
    }
    func matchesFavorites(_ preferences: SportsPreferences) -> Bool {
        event?.competitors?.contains { preferences.teams.contains($0.id) } == true
    }
}

enum SportsSchedule {
    static func date(_ raw: String?) -> Date? {
        guard let raw else { return nil }
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let result = formatter.date(from: raw) { return result }
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: raw)
    }
    static func calendar(timeZone: TimeZone) -> Calendar {
        var value = Calendar(identifier: .gregorian); value.timeZone = timeZone; return value
    }
    static func sameDay(_ date: Date, as selected: Date, timeZone: TimeZone) -> Bool {
        calendar(timeZone: timeZone).isDate(date, inSameDayAs: selected)
    }
    static func label(_ date: Date, timeZone: TimeZone, includeDate: Bool = true) -> String {
        let formatter = DateFormatter(); formatter.timeZone = timeZone
        formatter.dateStyle = includeDate ? .medium : .none; formatter.timeStyle = .short
        return formatter.string(from: date)
    }
    static func sorted(_ options: [SportsOption]) -> [SportsOption] {
        options.sorted {
            let lhs = $0.scheduleDate ?? .distantFuture, rhs = $1.scheduleDate ?? .distantFuture
            return lhs == rhs ? $0.id < $1.id : lhs < rhs
        }
    }
}
