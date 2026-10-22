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
            case .sessions(let daemon):
                selectDaemon(nodeId: daemon)
                chats.removeValue(forKey: daemon)
                preferences.set(daemon, forKey: "mew.lastDaemon")
            case .settings, .daemonDetails:
                continue
            }
            preferences.set(chats, forKey: "mew.lastChats")
            return
        }
        if path.isEmpty {
            clearSelection()
            preferences.removeObject(forKey: "mew.lastDaemon")
        }
    }

    func restoreWorkingLocation() {
        guard path.isEmpty, selectedDaemonId == nil else { return }
        if let saved = preferences.string(forKey: "mew.lastDaemon") {
            guard let daemon = daemons.first(where: { $0.nodeId == saved }) else {
                preferences.removeObject(forKey: "mew.lastDaemon")
                return
            }
            openDaemon(daemon)
        } else if daemons.count == 1, let daemon = daemons.first {
            openDaemon(daemon)
        }
    }

    func openDaemon(_ daemon: DaemonEntry) {
        guard daemons.contains(where: { $0.nodeId == daemon.nodeId }) else { return }
        let chats = preferences.dictionary(forKey: "mew.lastChats") as? [String: String] ?? [:]
        var destination: [NavigationRoute] = [.sessions(daemonNodeId: daemon.nodeId)]
        if let session = chats[daemon.nodeId],
           sessionLists[daemon.nodeId].map({ $0.contains { $0.sessionId == session } }) ?? true {
            destination.append(.chat(daemonNodeId: daemon.nodeId, sessionId: session))
        }
        path = destination
        if case .chat = destination.last { pendingRestoredChat = destination.last }
    }

    func validateRestoredChat(daemonNodeId: String, sessions: [SessionSummary]) {
        guard case .chat(let daemon, let session) = pendingRestoredChat,
              daemon == daemonNodeId else { return }
        pendingRestoredChat = nil
        if !sessions.contains(where: { $0.sessionId == session }) {
            path = [.sessions(daemonNodeId: daemon)]
        }
    }

    func forgetWorkingLocation(daemonNodeId: String) {
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
                case .sessions(let daemon), .chat(let daemon, _), .daemonDetails(let daemon):
                    return daemon == daemonNodeId
                case .settings:
                    return false
                }
            }
        }
        if preferences.string(forKey: "mew.lastDaemon") == daemonNodeId {
            preferences.removeObject(forKey: "mew.lastDaemon")
        }
    }
}
