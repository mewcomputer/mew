import Foundation
import MewMobileCore

/// A display fragment with the original message identities used by history anchors.
struct ConversationItem: Identifiable, Equatable {
    var message: ChatMessage
    var sourceMessageIds: [String]
    var id: String { message.id }

    static func grouped(messages: [ChatMessage], hidingPartId: String? = nil) -> [ConversationItem] {
        var items: [ConversationItem] = []
        for message in messages {
            guard message.role == "assistant" else {
                items.append(ConversationItem(message: message, sourceMessageIds: [message.id]))
                continue
            }
            for part in message.parts {
                if part.id == hidingPartId { continue }
                if part.kind == .text, part.text?.isEmpty != false { continue }
                let isTool = part.kind == .toolCall
                if let index = items.indices.last, items[index].message.role == "assistant",
                   (isTool && items[index].message.parts.first?.kind == .toolCall
                    || !isTool && items[index].sourceMessageIds.count == 1
                    && items[index].sourceMessageIds.first == message.id && items[index].message.parts.first?.kind != .toolCall) {
                    items[index].message.parts.append(part)
                    if items[index].sourceMessageIds.last != message.id {
                        items[index].sourceMessageIds.append(message.id)
                    }
                } else {
                    items.append(ConversationItem(message: ChatMessage(id: "part:\(part.id)", role: message.role,
                        parts: [part], assistantMeta: message.assistantMeta), sourceMessageIds: [message.id]))
                }
            }
        }
        items.removeAll { item in
            !item.message.parts.contains { part in
                switch part.kind {
                case .text, .reasoning, .error: return part.text?.isEmpty == false
                case .toolCall: return part.toolName != nil
                }
            }
        }
        return items
    }
}

/// Disclosure state follows tool identities when older history extends a group.
struct ToolDisclosures {
    private var expandedGroups = Set<String>()
    var expandedCalls = Set<String>()

    func isGroupExpanded(_ parts: [MessagePart]) -> Bool {
        parts.contains { expandedGroups.contains($0.id) }
    }

    mutating func setGroupExpanded(_ expanded: Bool, parts: [MessagePart]) {
        if expanded, let first = parts.first {
            expandedGroups.insert(first.id)
        } else {
            for part in parts { expandedGroups.remove(part.id) }
        }
    }
}

enum ToolActivity {
    private enum Kind { case edit, command, read, search, other }

    private static func kind(_ part: MessagePart) -> Kind {
        switch part.toolName {
        case "write", "edit", "edit_str_replace", "edit_hashline": return .edit
        case "bash", "shell", "shell_background", "shell_monitor": return .command
        case "read": return .read
        case "grep", "glob", "search": return .search
        default: return .other
        }
    }

    private static func input(_ part: MessagePart) -> [String: Any] {
        guard let data = part.toolInput?.data(using: .utf8),
              let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return [:] }
        return value
    }

    private static func paths(_ part: MessagePart, input: [String: Any]) -> [String] {
        if let path = input["path"] as? String, !path.isEmpty {
            return [(path as NSString).standardizingPath]
        }
        guard part.toolName == "edit_hashline", let patch = input["patch"] as? String else { return [] }
        return patch.split(whereSeparator: \.isNewline).compactMap { line in
            let line = line.trimmingCharacters(in: .whitespaces)
            guard line.hasPrefix("["), line.hasSuffix("]") else { return nil }
            let header = line.dropFirst().dropLast()
            guard let hash = header.lastIndex(of: "#"), hash != header.startIndex else { return nil }
            return (String(header[..<hash]) as NSString).standardizingPath
        }
    }

    fileprivate struct Metadata {
        let paths: [String]
        let command: String?
        let query: String?
    }

    /// Retains only display fields, avoiding repeated decoding of large write inputs.
    final class Cache {
        private struct Entry {
            let name: String?
            let input: String?
            let metadata: Metadata
        }
        private var entries: [String: Entry] = [:]

        fileprivate func metadata(for part: MessagePart) -> Metadata {
            if let entry = entries[part.id], entry.name == part.toolName, entry.input == part.toolInput {
                return entry.metadata
            }
            let input = ToolActivity.input(part)
            let metadata = Metadata(paths: ToolActivity.paths(part, input: input),
                command: input["command"] as? String, query: (input["pattern"] ?? input["query"]) as? String)
            entries[part.id] = Entry(name: part.toolName, input: part.toolInput, metadata: metadata)
            return metadata
        }
    }

    static func summary(_ parts: [MessagePart], cache: Cache = Cache()) -> String {
        var edits = 0, commands = 0, reads = 0, searches = 0, other = 0
        var editedPaths = Set<String>(), readPaths = Set<String>()
        var unknownEdits = false, unknownReads = false
        for part in parts {
            switch kind(part) {
            case .edit:
                edits += 1
                let targets = cache.metadata(for: part).paths
                unknownEdits = unknownEdits || targets.isEmpty
                editedPaths.formUnion(targets)
            case .command: commands += 1
            case .read:
                reads += 1
                let targets = cache.metadata(for: part).paths
                unknownReads = unknownReads || targets.isEmpty
                readPaths.formUnion(targets)
            case .search: searches += 1
            case .other: other += 1
            }
        }
        let completed = parts.allSatisfy { $0.toolState == "completed" }
        var phrases: [String] = []
        if edits > 0 {
            if unknownEdits {
                phrases.append("\(completed ? "made " : "")\(edits) \(edits == 1 ? "edit" : "edits")")
            } else {
                phrases.append(completed
                    ? "edited \(editedPaths.count) \(editedPaths.count == 1 ? "file" : "files")"
                    : "\(editedPaths.count) \(editedPaths.count == 1 ? "file edit" : "file edits")")
            }
        }
        if commands > 0 { phrases.append("\(completed ? "ran " : "")\(commands) \(commands == 1 ? "command" : "commands")") }
        if reads > 0 {
            if unknownReads {
                phrases.append(completed ? "read \(reads) \(reads == 1 ? "time" : "times")" : "\(reads) reads")
            } else {
                phrases.append(completed
                    ? "read \(readPaths.count) \(readPaths.count == 1 ? "file" : "files")"
                    : "\(readPaths.count) \(readPaths.count == 1 ? "file read" : "file reads")")
            }
        }
        if searches > 0 { phrases.append("\(completed ? "ran " : "")\(searches) \(searches == 1 ? "search" : "searches")") }
        if other > 0 { phrases.append("\(completed ? "made " : "")\(other) \(other == 1 ? "tool call" : "tool calls")") }
        guard let last = phrases.popLast() else { return "Tool activity" }
        let title = phrases.isEmpty ? last : phrases.joined(separator: ", ") + (phrases.count > 1 ? ", and " : " and ") + last
        return title.prefix(1).uppercased() + title.dropFirst()
    }

    static func status(_ parts: [MessagePart]) -> String {
        var running = 0, failed = 0, pending = 0
        for part in parts {
            switch part.toolState {
            case "running": running += 1
            case "error": failed += 1
            case "completed": break
            default: pending += 1
            }
        }
        var labels: [String] = []
        if running > 0 { labels.append("\(running) running") }
        if pending > 0 { labels.append("\(pending) pending") }
        if failed > 0 { labels.append("\(failed) failed") }
        return labels.isEmpty ? "Completed" : labels.joined(separator: ", ")
    }

    static func title(_ part: MessagePart, cache: Cache = Cache()) -> String {
        let metadata = cache.metadata(for: part)
        let completed = part.toolState == "completed"
        let running = part.toolState == "running"
        let failed = part.toolState == "error"
        switch kind(part) {
        case .command:
            if let command = metadata.command, !command.isEmpty {
                return "\(completed ? "Ran" : running ? "Running" : failed ? "Command failed:" : "Run") \(command)"
            }
        case .edit, .read:
            let targets = metadata.paths
            if !targets.isEmpty {
                let count = targets.count > 1 ? Set(targets).count : 1
                let target = count == 1 ? (targets[0] as NSString).lastPathComponent : "\(count) files"
                if kind(part) == .edit {
                    return "\(completed ? "Edited" : running ? "Editing" : failed ? "Edit failed:" : "Edit") \(target)"
                }
                return "\(running ? "Reading" : failed ? "Read failed:" : "Read") \(target)"
            }
        case .search:
            if let query = metadata.query, !query.isEmpty {
                return "\(completed ? "Searched for" : running ? "Searching for" : failed ? "Search failed:" : "Search for") \(query)"
            }
        case .other: break
        }
        return part.toolName ?? "Tool"
    }

    static func symbol(_ part: MessagePart) -> String {
        switch kind(part) {
        case .edit: return "pencil"
        case .command: return "terminal"
        case .read: return "doc.text"
        case .search: return "magnifyingglass"
        case .other: return "wrench.and.screwdriver"
        }
    }
}
