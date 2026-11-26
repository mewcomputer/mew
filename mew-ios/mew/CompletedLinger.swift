import Foundation

/// Remembers when work was first seen finished so completed checklists and
/// background jobs can leave the screen after a short grace instead of
/// lingering for the rest of the session.
struct CompletedLinger {
    /// How long finished work keeps its place on screen.
    static let grace: TimeInterval = 20

    private var finishedAt: [String: Date] = [:]

    /// Record which keys are finished at `now`, forgetting the ones that are
    /// no longer finished so a restarted task becomes visible again.
    mutating func observe(finished keys: Set<String>, now: Date) {
        for key in keys where finishedAt[key] == nil {
            finishedAt[key] = now
        }
        finishedAt = finishedAt.filter { keys.contains($0.key) }
    }

    /// Whether finished work still deserves its place on screen.
    func isVisible(_ key: String, now: Date) -> Bool {
        guard let finished = finishedAt[key] else { return true }
        return now.timeIntervalSince(finished) < Self.grace
    }

    /// Whether any recorded work is still waiting out the grace window, so the
    /// caller knows another prune is worth scheduling.
    func hasFinishedWorkInsideGrace(now: Date) -> Bool {
        finishedAt.values.contains { now.timeIntervalSince($0) < Self.grace }
    }
}
