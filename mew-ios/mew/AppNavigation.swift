import Foundation
import MewMobileCore

extension AppStore {
    func synchronizeNavigation() {
        if pendingRestoredChat != path.last { pendingRestoredChat = nil }
        for route in path.reversed() {
            var chats = preferences.dictionary(forKey: "mew.lastChats") as? [String: String] ?? [:]
            switch route {
            case .chat(let daemon, let session):
                selectSession(session, daemonNodeId: daemon)
                chats[daemon] = session
                preferences.set(daemon, forKey: "mew.lastDaemon")
            case .machines, .settings, .daemonDetails:
                continue
            }
            preferences.set(chats, forKey: "mew.lastChats")
            return
        }
        clearSelection()
        if path.isEmpty {
            preferences.removeObject(forKey: "mew.lastDaemon")
            refreshSessions(daemonNodeId: sessionFilter)
        }
    }

    func restoreWorkingLocation() {
        if let sessionFilter, !daemons.contains(where: { $0.nodeId == sessionFilter }) {
            self.sessionFilter = nil
        }
        guard path.isEmpty, selectedDaemonId == nil,
              let saved = preferences.string(forKey: "mew.lastDaemon") else { return }
        guard daemons.contains(where: { $0.nodeId == saved }) else {
            preferences.removeObject(forKey: "mew.lastDaemon")
            return
        }
        restoreChat(daemonNodeId: saved)
    }

    func openDaemon(_ daemon: DaemonEntry) {
        guard daemons.contains(where: { $0.nodeId == daemon.nodeId }) else { return }
        sessionFilter = daemon.nodeId
        restoreChat(daemonNodeId: daemon.nodeId)
    }

    private func restoreChat(daemonNodeId: String) {
        let chats = preferences.dictionary(forKey: "mew.lastChats") as? [String: String] ?? [:]
        if let session = chats[daemonNodeId],
           sessionLists[daemonNodeId].map({ $0.contains { $0.sessionId == session } }) ?? true {
            path = [.chat(daemonNodeId: daemonNodeId, sessionId: session)]
            pendingRestoredChat = path.last
        } else {
            path = []
        }
    }

    func validateRestoredChat(daemonNodeId: String, sessions: [SessionSummary]) {
        guard case .chat(let daemon, let session) = pendingRestoredChat,
              daemon == daemonNodeId else { return }
        pendingRestoredChat = nil
        if !sessions.contains(where: { $0.sessionId == session }) {
            path = []
        }
    }

    func forgetWorkingLocation(daemonNodeId: String) {
        if sessionFilter == daemonNodeId { sessionFilter = nil }
        var chats = preferences.dictionary(forKey: "mew.lastChats") as? [String: String] ?? [:]
        chats.removeValue(forKey: daemonNodeId)
        preferences.set(chats, forKey: "mew.lastChats")
        drafts = drafts.filter { route, _ in
            if case .chat(let daemon, _) = route { return daemon != daemonNodeId }
            return true
        }
        if selectedDaemonId?.nodeId == daemonNodeId {
            path = []
        } else {
            path.removeAll { route in
                switch route {
                case .chat(let daemon, _), .daemonDetails(let daemon):
                    return daemon == daemonNodeId
                case .machines, .settings:
                    return false
                }
            }
        }
        if preferences.string(forKey: "mew.lastDaemon") == daemonNodeId {
            preferences.removeObject(forKey: "mew.lastDaemon")
        }
    }
}
