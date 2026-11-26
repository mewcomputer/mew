import SwiftUI
import WebKit
import MewMobileCore
import SwiftStreamingMarkdown

// MARK: - MessageItemView

/// Renders a single `ChatMessage` from the conversation.
///
/// User messages are right-aligned accent bubbles; assistant messages are
/// left-aligned with a subtle background. Each `MessagePart` is rendered
/// according to its `kind`:
/// - `.text`: markdown via `AttributedString(markdown:)`
/// - `.reasoning`: collapsible muted disclosure group
/// - `.toolCall`: compact row with a state indicator and expandable input/output
/// - `.error`: red text with a warning icon
struct MessageItemView: View {
    let message: ChatMessage
    var fontChoice: MewFontChoice = .instrumentSans
    var onOpenFile: ((String) -> Void)? = nil
    var onOpenArtifact: ((MessagePart) -> Void)? = nil
    @Binding var disclosures: ToolDisclosures
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme
    @State private var copiedText = false

    private var isUser: Bool { message.role == "user" }
    private var isSystem: Bool { message.role == "system" }

    var body: some View {
        Group {
            if isSystem {
                systemContent
            } else {
                HStack {
                    if isUser {
                        Spacer(minLength: 48)
                        userBubble
                    } else {
                        assistantContent
                    }
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 3)
    }

    // MARK: User

    @ViewBuilder
    private var userBubble: some View {
        VStack(alignment: .trailing, spacing: 4) {
            if message.steered {
                Text("Steered")
                    .font(.caption2.weight(.semibold))
                    .foregroundStyle(.secondary)
                    .padding(.horizontal, 8)
                    .padding(.vertical, 2)
                    .background(Color(.tertiarySystemFill), in: Capsule())
            }
            VStack(alignment: .trailing, spacing: 6) {
                ForEach(message.parts, id: \.id) { part in
                    if part.kind == .text, let text = part.text, !text.isEmpty {
                        Text(markdown(text))
                            .font(fontChoice.swiftUIFont(UIFont.systemFontSize))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                    } else if part.kind == .error, let text = part.text {
                        Text(text)
                            .font(.body)
                            .foregroundStyle(.red)
                    }
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(theme.userBubble, in: RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous))
            .overlay(
                RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous)
                    .strokeBorder(theme.border.opacity(0.6), lineWidth: 0.5)
            )
        }
    }

    // MARK: System

    @ViewBuilder
    private var systemContent: some View {
        VStack(spacing: 6) {
            ForEach(message.parts, id: \.id) { part in
                partView(part)
            }
        }
        .frame(maxWidth: .infinity)
    }

    // MARK: Assistant

    private var isWorkOnly: Bool {
        !message.parts.isEmpty
            && message.parts.contains { $0.kind == .toolCall || $0.kind == .reasoning }
            && !message.parts.contains { $0.kind == .error || $0.kind == .compaction }
    }

    private var copyableText: String {
        message.parts
            .filter { $0.kind == .text }
            .compactMap(\.text)
            .filter { !$0.isEmpty }
            .joined(separator: "\n\n")
    }

    @ViewBuilder
    private var assistantContent: some View {
        VStack(alignment: .leading, spacing: 4) {
            if isWorkOnly {
                WorkGroupRow(parts: message.parts, fontChoice: fontChoice, disclosures: $disclosures)
            } else {
                ForEach(message.parts, id: \.id) { part in
                    partView(part)
                }
                if !copyableText.isEmpty && !message.cancelled {
                    HStack(spacing: 12) {
                        Button {
                            copyAssistantText()
                        } label: {
                            Label(copiedText ? "Copied" : "Copy", systemImage: copiedText ? "checkmark" : "doc.on.doc")
                                .font(.caption2.weight(.medium))
                                .foregroundStyle(copiedText ? Color.green : Color.secondary)
                                .padding(.vertical, 4)
                                .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .accessibilityLabel("Copy response")
                    }
                    .padding(.top, 2)
                }
            }
            if message.cancelled {
                Label("Cancelled", systemImage: "stop.circle")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .padding(.top, 2)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, isWorkOnly ? 0 : 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        // Assistant messages render with no background — the chat surface
        // shows through and user bubbles provide the only visual separation.
    }

    private func copyAssistantText() {
        UIPasteboard.general.string = copyableText
        UINotificationFeedbackGenerator().notificationOccurred(.success)
        withAnimation(Theme.Motion.value(Theme.Motion.press, reduced: reduceMotion)) {
            copiedText = true
        }
        Task {
            try? await Task.sleep(nanoseconds: 1_500_000_000)
            await MainActor.run {
                withAnimation(Theme.Motion.value(Theme.Motion.press, reduced: reduceMotion)) {
                    copiedText = false
                }
            }
        }
    }

    @ViewBuilder
    private func partView(_ part: MessagePart) -> some View {
        switch part.kind {
        case .text:
            if let text = part.text, !text.isEmpty {
                MarkdownView(text: FileDirective.markdown(text), config: .mewStatic(fontChoice: fontChoice))
                    .textSelection(.enabled)
                    .fileReferenceOpenURL(onOpenFile: onOpenFile)
            }
        case .reasoning:
            reasoningView(part)
        case .toolCall:
            toolCallView(part)
        case .compaction:
            CompactionDividerView(part: part, fontChoice: fontChoice)
        case .file:
            fileAttachmentView(part)
        case .artifact:
            artifactView(part)
        case .error:
            if let text = part.text, !text.isEmpty {
                Label(text, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.red)
                    .textSelection(.enabled)
            }
        }
    }

    @ViewBuilder
    private func fileAttachmentView(_ part: MessagePart) -> some View {
        let label = part.fileName ?? part.text ?? part.fileMime ?? "Attachment"
        let isImage = part.fileMime?.hasPrefix("image/") == true
        let shape = RoundedRectangle(cornerRadius: 10, style: .continuous)
        HStack(spacing: 8) {
            Image(systemName: isImage ? "photo" : "doc.fill")
                .font(.caption)
                .foregroundStyle(theme.textAccent)
            Text(label)
                .font(.caption.weight(.medium))
                .foregroundStyle(theme.foreground)
                .lineLimit(1)
                .truncationMode(.middle)
            if let mime = part.fileMime, !mime.isEmpty, mime != label {
                Text(mime)
                    .font(.caption2)
                    .foregroundStyle(theme.mutedForeground)
                    .lineLimit(1)
            }
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(theme.toolBackground, in: shape)
        .overlay(shape.strokeBorder(theme.toolBorder, lineWidth: 0.5))
    }

    @ViewBuilder
    private func artifactView(_ part: MessagePart) -> some View {
        let title = part.text ?? "Artifact"
        if part.artifactMode == "inline" {
            ArtifactInlineCardView(
                part: part,
                onOpenPanel: onOpenArtifact.map { cb in { cb(part) } }
            )
        } else {
            Button {
                onOpenArtifact?(part)
            } label: {
                HStack(spacing: 6) {
                    Text("open \(title)")
                        .font(.callout.weight(.medium))
                        .foregroundStyle(theme.foreground)
                        .lineLimit(1)
                    Image(systemName: "arrow.up.right")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(theme.mutedForeground)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(theme.toolBackground, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: 10, style: .continuous)
                        .strokeBorder(theme.toolBorder, lineWidth: 0.5)
                )
            }
            .buttonStyle(.plain)
        }
    }

    @ViewBuilder
    private func reasoningView(_ part: MessagePart) -> some View {
        let text = part.text ?? ""
        DisclosureGroup {
            if !text.isEmpty {
                MarkdownView(text: text, config: .mewReasoning(fontChoice: fontChoice))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        } label: {
            Label("Reasoning", systemImage: "brain.head.profile")
                .font(.footnote.weight(.medium))
                .foregroundStyle(.secondary)
        }
        .tint(.secondary)
    }

    @ViewBuilder
    private func toolCallView(_ part: MessagePart) -> some View {
        ToolCallRow(part: part, expandedCalls: $disclosures.expandedCalls)
    }

    // MARK: Helpers

    /// Parses inline markdown into an `AttributedString`, falling back to the
    /// raw text if parsing fails. No third-party markdown dependency.
    private func markdown(_ string: String) -> AttributedString {
        if let attr = try? AttributedString(markdown: string) {
            return attr
        }
        return AttributedString(string)
    }
}

// MARK: - File reference links

private extension View {
    /// Route file reference pills to the chat's preview sheet, leaving normal
    /// markdown links to the system.
    func fileReferenceOpenURL(onOpenFile: ((String) -> Void)?) -> some View {
        environment(\.openURL, OpenURLAction { url in
            guard let path = FileDirective.path(from: url), let onOpenFile else {
                return .systemAction
            }
            onOpenFile(path)
            return .handled
        })
    }
}

// MARK: - Work group row

/// Consecutive reasoning and tool activity at one conversation level, folded into
/// a single disclosure row with a subtle left rail when expanded.
private struct WorkGroupRow: View {
    let parts: [MessagePart]
    let fontChoice: MewFontChoice
    @Binding var disclosures: ToolDisclosures

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme
    @State private var activityCache = ToolActivity.Cache()
    private var expanded: Bool { disclosures.isGroupExpanded(parts) }

    private var toolParts: [MessagePart] {
        parts.filter { $0.kind == .toolCall }
    }
    private var anyActive: Bool {
        toolParts.contains { $0.toolState != "completed" && $0.toolState != "error" }
    }
    private var anyError: Bool {
        toolParts.contains { $0.toolState == "error" }
    }
    private var leadSymbol: String {
        if let firstTool = toolParts.first {
            return ToolActivity.symbol(firstTool)
        }
        return "brain.head.profile"
    }

    var body: some View {
        let summary = ToolActivity.workSummary(parts, cache: activityCache)
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(Theme.Motion.value(.spring(response: 0.3, dampingFraction: 1.0), reduced: reduceMotion)) {
                    disclosures.setGroupExpanded(!expanded, parts: parts)
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: leadSymbol)
                        .font(.subheadline)
                    Text(summary)
                        .font(.subheadline)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                    Spacer(minLength: 4)
                    if !toolParts.isEmpty {
                        aggregateIndicator
                    }
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.caption2)
                }
                .foregroundStyle(.secondary)
                .frame(minHeight: 40)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(summary)
            .accessibilityValue(ToolActivity.status(parts))
            .accessibilityHint(expanded ? "Hide work details" : "Show work details")

            if expanded {
                HStack(alignment: .top, spacing: 10) {
                    Rectangle()
                        .fill(Color(.separator).opacity(0.6))
                        .frame(width: 1.5)
                    VStack(alignment: .leading, spacing: 6) {
                        ForEach(parts, id: \.id) { part in
                            if part.kind == .reasoning {
                                reasoningRow(part)
                            } else if part.kind == .toolCall {
                                ToolCallRow(part: part, expandedCalls: $disclosures.expandedCalls)
                            } else if part.kind == .text, let text = part.text,
                                      !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                                MarkdownView(text: text, config: .mewReasoning(fontChoice: fontChoice))
                                    .textSelection(.enabled)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                            }
                        }
                    }
                }
                .padding(.leading, 6)
                .padding(.bottom, 4)
            }
        }
    }

    @ViewBuilder
    private func reasoningRow(_ part: MessagePart) -> some View {
        let text = part.text ?? ""
        DisclosureGroup {
            if !text.isEmpty {
                MarkdownView(text: text, config: .mewReasoning(fontChoice: fontChoice))
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        } label: {
            Label("Reasoning", systemImage: "brain.head.profile")
                .font(.footnote.weight(.medium))
                .foregroundStyle(.secondary)
        }
        .tint(.secondary)
    }

    @ViewBuilder
    private var aggregateIndicator: some View {
        HStack(spacing: 6) {
            if anyActive {
                ProgressView().controlSize(.small).tint(theme.textAccent)
            }
            if anyError {
                Image(systemName: "xmark.circle.fill").foregroundStyle(theme.textError)
            } else if !anyActive {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(theme.textSuccess)
            }
        }
        .font(.caption)
        .accessibilityHidden(true)
    }
}

// MARK: - Tool call row

struct ToolHistoryView: View {
    let messages: [ChatMessage]

    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme
    @State private var search = ""
    @State private var expandedCalls = Set<String>()

    var body: some View {
        let tools = Array(messages.lazy.flatMap(\.parts).filter { part in
            guard part.kind == .toolCall else { return false }
            return search.isEmpty
                || part.toolName?.localizedCaseInsensitiveContains(search) == true
                || part.toolInput?.localizedCaseInsensitiveContains(search) == true
                || part.toolOutput?.localizedCaseInsensitiveContains(search) == true
                || part.toolError?.localizedCaseInsensitiveContains(search) == true
        })
        NavigationStack {
            Group {
                if tools.isEmpty {
                    ContentUnavailableView(search.isEmpty ? "No tool calls" : "No matching tools",
                        systemImage: "wrench.and.screwdriver",
                        description: Text(search.isEmpty ? "Tool activity from this conversation appears here." : "Try a tool name or text from its input or output."))
                } else {
                    List(tools, id: \.id) { part in
                        ToolCallRow(part: part, expandedCalls: $expandedCalls)
                            .listRowInsets(EdgeInsets(top: 0, leading: 16, bottom: 0, trailing: 16))
                            .listRowBackground(Color.clear)
                    }
                    .listStyle(.plain)
                    .mewThemedPlainList(theme)
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(theme.background.ignoresSafeArea())
            .navigationTitle("Tools")
            .navigationBarTitleDisplayMode(.inline)
            .searchable(text: $search, prompt: "Search tools, input, and output")
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }
}

/// Compact, expandable row representing a `.toolCall` part.
private struct ToolCallRow: View {
    let part: MessagePart
    @Binding var expandedCalls: Set<String>

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme
    @State private var activityCache = ToolActivity.Cache()
    private var expanded: Bool { expandedCalls.contains(part.id) }

    private var state: ToolState {
        switch part.toolState ?? "" {
        case "pending":    return .pending
        case "running":    return .running
        case "completed":  return .completed
        case "error":      return .error
        default:           return .pending
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(Theme.Motion.value(Theme.Motion.disclosure, reduced: reduceMotion)) {
                    if expanded { expandedCalls.remove(part.id) } else { expandedCalls.insert(part.id) }
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: ToolActivity.symbol(part))
                        .font(.subheadline)
                        .foregroundStyle(.secondary)

                    Text(ToolActivity.title(part, cache: activityCache))
                        .font(.subheadline.weight(.medium))
                        .lineLimit(1)

                    Spacer(minLength: 4)

                    stateIndicator

                    if part.toolState == "running" {
                        TimelineView(.periodic(from: .now, by: 1)) { context in
                            durationLabel(now: context.date)
                        }
                    } else if part.toolTimeEnd != nil {
                        durationLabel(now: Date())
                    }

                    Image(systemName: expanded ? "chevron.up" : "chevron.down")
                        .font(.caption2)
                        .foregroundStyle(.tertiary)
                }
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityValue(part.toolState ?? "pending")
            .accessibilityHint(expanded ? "Hide input and output" : "Show input and output")

            if expanded {
                detailContent
            }
        }
    }

    @ViewBuilder
    private func durationLabel(now: Date) -> some View {
        if let duration = FormatHelper.toolDuration(start: part.toolTimeStart, end: part.toolTimeEnd, now: now) {
            Text(duration)
                .font(.caption.monospacedDigit())
                .foregroundStyle(.secondary)
        }
    }

    @ViewBuilder
    private var detailContent: some View {
        VStack(alignment: .leading, spacing: 8) {
            // Input
            if let input = part.toolInput, !input.isEmpty {
                if let diffLines = inlineDiffLines(toolName: part.toolName, rawInput: input) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Changes")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        ScrollView(.vertical) {
                            LazyVStack(alignment: .leading, spacing: 0) {
                                ForEach(Array(diffLines.enumerated()), id: \.offset) { _, line in
                                    Text(line.text)
                                        .font(.mewMono(12))
                                        .foregroundStyle(line.color)
                                        .frame(maxWidth: .infinity, alignment: .leading)
                                        .padding(.horizontal, 8)
                                        .padding(.vertical, 2)
                                        .background(line.background)
                                }
                            }
                        }
                        .frame(maxHeight: 220)
                        .textSelection(.enabled)
                        .background(theme.toolBackground, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(theme.toolBorder, lineWidth: 0.5))
                        .clipShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
                    }
                } else {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("Input")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text(FormatHelper.prettyJSON(input))
                            .font(.mewMono(12))
                            .foregroundStyle(.primary)
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(8)
                            .background(theme.toolBackground, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                            .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(theme.toolBorder, lineWidth: 0.5))
                    }
                }
            }

            // Output (completed or running)
            if let output = part.toolOutput, !output.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Output")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Text(output)
                        .font(.mewMono(12))
                        .foregroundStyle(.primary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                        .background(theme.toolBackground, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                        .overlay(RoundedRectangle(cornerRadius: 8, style: .continuous).strokeBorder(theme.toolBorder, lineWidth: 0.5))
                }
            }

            // Error
            if let error = part.toolError, !error.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Error")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.red)
                    Text(error)
                        .font(.mewMono(12))
                        .foregroundStyle(.red)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                        .background(Color.red.opacity(0.08), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }
        }
        .padding(.top, 2)
    }

    @ViewBuilder
    private var stateIndicator: some View {
        switch state {
        case .pending:
            Image(systemName: "circle")
                .foregroundStyle(theme.mutedForeground)
                .font(.caption)
        case .running:
            ProgressView()
                .controlSize(.small)
                .tint(theme.textAccent)
        case .completed:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(theme.textSuccess)
                .font(.caption)
        case .error:
            Image(systemName: "xmark.circle.fill")
                .foregroundStyle(theme.textError)
                .font(.caption)
        }
    }

    private enum ToolState { case pending, running, completed, error }

    private struct DiffLine {
        let text: String
        let color: Color
        let background: Color
    }

    private func inlineDiffLines(toolName: String?, rawInput: String) -> [DiffLine]? {
        guard let data = rawInput.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return nil
        }
        if toolName == "edit_hashline", let patch = json["patch"] as? String, !patch.isEmpty {
            let lines = patch.components(separatedBy: .newlines).map { line -> DiffLine in
                if line.hasPrefix("+") {
                    return DiffLine(text: line, color: .green, background: Color.green.opacity(0.10))
                } else if line.hasPrefix("-") || line.hasPrefix("DEL ") {
                    return DiffLine(text: line, color: .red, background: Color.red.opacity(0.10))
                } else if line.hasPrefix("[") && line.hasSuffix("]") {
                    return DiffLine(text: line, color: .secondary, background: Color(.secondarySystemBackground))
                }
                return DiffLine(text: line, color: .primary, background: .clear)
            }
            return lines.isEmpty ? nil : lines
        }
        let oldStr = (json["old_str"] as? String) ?? (json["old_string"] as? String)
        let newStr = (json["new_str"] as? String) ?? (json["new_string"] as? String)
        if let oldStr, let newStr {
            var lines: [DiffLine] = []
            for line in oldStr.components(separatedBy: .newlines) {
                lines.append(DiffLine(text: "− \(line)", color: .red, background: Color.red.opacity(0.10)))
            }
            for line in newStr.components(separatedBy: .newlines) {
                lines.append(DiffLine(text: "+ \(line)", color: .green, background: Color.green.opacity(0.10)))
            }
            return lines.isEmpty ? nil : lines
        }
        return nil
    }
}

// MARK: - Streaming bubble

/// Bridges the accumulating streaming text into the `AsyncStream<String>` that
/// `StreamedMarkdownView` consumes. Each yield is the complete snapshot so far;
/// the newest-wins buffer drops intermediate snapshots the parser can't keep up
/// with. Owned as a `@StateObject` so its lifetime matches the bubble.
final class LiveMarkdownSource: ObservableObject, StreamedMarkdownSource {
    let text: AsyncStream<String>
    private let continuation: AsyncStream<String>.Continuation

    init() {
        var captured: AsyncStream<String>.Continuation!
        text = AsyncStream(bufferingPolicy: .bufferingNewest(1)) { captured = $0 }
        continuation = captured
    }

    func yield(_ snapshot: String) {
        continuation.yield(snapshot)
    }

    deinit {
        continuation.finish()
    }
}

/// Assistant-style bubble for live streaming text that hasn't been committed to
/// a `ChatMessage` yet. Renders incrementally via `StreamedMarkdownView` so
/// blocks stay stable and newly appended text fades in as it arrives.
struct StreamingBubble: View {
    let text: String
    var fontChoice: MewFontChoice = .instrumentSans
    var onOpenFile: ((String) -> Void)? = nil
    @StateObject private var source = LiveMarkdownSource()

    var body: some View {
        VStack(alignment: .leading, spacing: 6) {
            StreamedMarkdownView(source: source, config: .mew(fontChoice: fontChoice))
                .textSelection(.enabled)
                .fileReferenceOpenURL(onOpenFile: onOpenFile)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, 16)
        .padding(.vertical, 3)
        .onChange(of: text, initial: true) { _, newValue in
            source.yield(FileDirective.markdown(newValue))
        }
    }
}

private struct CompactionDividerView: View {
    let part: MessagePart
    var fontChoice: MewFontChoice = .instrumentSans
    @State private var expanded = false

    private var labelText: String {
        if let removed = part.compactionRemoved, removed > 0 {
            let unit = removed == 1 ? "message" : "messages"
            return "Context compacted · \(removed) \(unit)"
        }
        return "Context compacted"
    }

    private var summary: String {
        (part.text ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private var headerRow: some View {
        HStack(spacing: 8) {
            Rectangle()
                .fill(Color.secondary.opacity(0.25))
                .frame(height: 1)
            Text(labelText)
                .font(.caption.weight(.medium))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 10)
                .padding(.vertical, 3)
                .background(Color(.tertiarySystemFill), in: Capsule())
            Rectangle()
                .fill(Color.secondary.opacity(0.25))
                .frame(height: 1)
        }
    }

    var body: some View {
        VStack(spacing: 8) {
            if summary.isEmpty {
                headerRow
            } else {
                DisclosureGroup(isExpanded: $expanded) {
                    MarkdownView(text: summary, config: .mewReasoning(fontChoice: fontChoice))
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(10)
                        .background(
                            Color(.secondarySystemBackground),
                            in: RoundedRectangle(cornerRadius: 10, style: .continuous)
                        )
                } label: {
                    headerRow
                }
                .tint(.secondary)
            }
        }
        .padding(.vertical, 4)
    }
}

// MARK: - Artifact WebView & Sheet

struct ArtifactSheetItem: Identifiable {
    let part: MessagePart
    var id: String { part.id }
}

private struct ArtifactInlineCardView: View {
    let part: MessagePart
    let onOpenPanel: (() -> Void)?

    @EnvironmentObject private var store: AppStore
    @Environment(\.mewTheme) private var theme
    @State private var inlineHeight: CGFloat

    init(part: MessagePart, onOpenPanel: (() -> Void)?) {
        self.part = part
        self.onOpenPanel = onOpenPanel
        let initial = part.artifactHeight.map { CGFloat($0) } ?? 320
        _inlineHeight = State(initialValue: min(max(initial, 120), 900))
    }

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: 12, style: .continuous)
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Text(part.text ?? "Artifact")
                    .font(.caption.weight(.semibold))
                    .foregroundStyle(theme.foreground)
                    .lineLimit(1)
                Spacer()
                if let onOpenPanel {
                    Button {
                        onOpenPanel()
                    } label: {
                        HStack(spacing: 3) {
                            Text("Open")
                            Image(systemName: "arrow.up.right")
                        }
                        .font(.caption2.weight(.medium))
                        .foregroundStyle(theme.mutedForeground)
                    }
                    .buttonStyle(.plain)
                }
            }
            .padding(.horizontal, 12)
            .padding(.vertical, 8)
            .background(theme.toolBackground)

            Divider()

            ArtifactWebView(
                urlPath: part.artifactUrl ?? "",
                isInline: true,
                onResize: { next in
                    let clamped = min(max(next, 120), 900)
                    if abs(clamped - inlineHeight) > 1 {
                        inlineHeight = clamped
                    }
                }
            )
            .environmentObject(store)
            .frame(height: inlineHeight)
        }
        .clipShape(shape)
        .overlay(shape.strokeBorder(theme.toolBorder, lineWidth: 0.75))
    }
}

struct ArtifactSheetView: View {
    let part: MessagePart
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var store: AppStore
    @Environment(\.mewTheme) private var theme
    @State private var dragOffset: CGFloat = 0

    var body: some View {
        VStack(spacing: 0) {
            ZStack {
                Text(part.text ?? "Artifact")
                    .font(.headline)
                    .foregroundStyle(theme.foreground)
                    .lineLimit(1)
                    .padding(.horizontal, 68)
                HStack {
                    Button("Done") { dismiss() }
                        .font(.body)
                    Spacer()
                }
            }
            .padding(.horizontal, 16)
            .padding(.top, 14)
            .padding(.bottom, 12)
            .background(theme.toolBackground)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 6, coordinateSpace: .global)
                    .onChanged { value in
                        if value.translation.height > 0 {
                            dragOffset = value.translation.height
                        }
                    }
                    .onEnded { value in
                        if value.translation.height > 110 || value.predictedEndTranslation.height > 200 {
                            dismiss()
                        } else {
                            withAnimation(.spring(response: 0.28, dampingFraction: 0.86)) {
                                dragOffset = 0
                            }
                        }
                    }
            )

            Divider()

            ArtifactWebView(urlPath: part.artifactUrl ?? "", isInline: false, onResize: nil)
                .environmentObject(store)
                .ignoresSafeArea(edges: .bottom)
        }
        .offset(y: dragOffset)
        .interactiveDismissDisabled(true)
    }
}

private struct ArtifactWebView: UIViewRepresentable {
    let urlPath: String
    let isInline: Bool
    let onResize: ((CGFloat) -> Void)?

    @EnvironmentObject private var store: AppStore
    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.mewTheme) private var theme

    func makeCoordinator() -> Coordinator {
        Coordinator(store: store, onResize: onResize)
    }

    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.setURLSchemeHandler(context.coordinator, forURLScheme: "mew-artifact")
        let controller = config.userContentController
        controller.add(context.coordinator, name: "mewResize")
        let bridgeScript = """
        window.addEventListener("message", function(event) {
          var d = event && event.data;
          if (!d || (d.type !== "mew:artifact-resize" && d.type !== "mew:resize")) return;
          if (typeof d.height === "number" && isFinite(d.height) && window.webkit && window.webkit.messageHandlers && window.webkit.messageHandlers.mewResize) {
            window.webkit.messageHandlers.mewResize.postMessage(d.height);
          }
        });
        """
        controller.addUserScript(WKUserScript(source: bridgeScript, injectionTime: .atDocumentStart, forMainFrameOnly: true))
        let webView = WKWebView(frame: .zero, configuration: config)
        webView.navigationDelegate = context.coordinator
        webView.isOpaque = false
        webView.backgroundColor = .clear
        webView.scrollView.isScrollEnabled = !isInline
        context.coordinator.themePayload = themePayloadJSON()
        loadArtifact(in: webView, coordinator: context.coordinator)
        return webView
    }

    func updateUIView(_ webView: WKWebView, context: Context) {
        context.coordinator.onResize = onResize
        let nextTheme = themePayloadJSON()
        if context.coordinator.themePayload != nextTheme {
            context.coordinator.themePayload = nextTheme
            webView.evaluateJavaScript("window.postMessage(\(nextTheme), '*');", completionHandler: nil)
        }
        if context.coordinator.loadedPath != urlPath {
            loadArtifact(in: webView, coordinator: context.coordinator)
        }
    }

    private func loadArtifact(in webView: WKWebView, coordinator: Coordinator) {
        coordinator.loadedPath = urlPath
        guard urlPath.hasPrefix("/artifacts/"),
              let url = URL(string: "mew-artifact://app\(urlPath)") else {
            webView.loadHTMLString("<p style='font-family:sans-serif;padding:12px;'>Invalid artifact URL.</p>", baseURL: nil)
            return
        }
        webView.load(URLRequest(url: url))
    }

    private func themePayloadJSON() -> String {
        let isDark = colorScheme == .dark
        let bg = theme.background.hexString ?? (isDark ? "#18181b" : "#ffffff")
        let fg = theme.foreground.hexString ?? (isDark ? "#f4f4f5" : "#18181b")
        let muted = theme.mutedForeground.hexString ?? "#71717a"
        let card = theme.toolBackground.hexString ?? (isDark ? "#27272a" : "#f4f4f5")
        let border = theme.toolBorder.hexString ?? (isDark ? "#3f3f46" : "#e4e4e7")
        let primary = theme.textAccent.hexString ?? "#2563eb"
        let payload: [String: Any] = [
            "type": "mew:theme",
            "isDark": isDark,
            "theme": isDark ? "dark" : "light",
            "vars": [
                "--background": bg,
                "--foreground": fg,
                "--card": card,
                "--card-foreground": fg,
                "--muted": card,
                "--muted-foreground": muted,
                "--border": border,
                "--input": border,
                "--primary": primary,
                "--primary-foreground": "#ffffff",
            ]
        ]
        guard let data = try? JSONSerialization.data(withJSONObject: payload),
              let json = String(data: data, encoding: .utf8) else {
            return "{}"
        }
        return json
    }

    final class Coordinator: NSObject, WKURLSchemeHandler, WKScriptMessageHandler, WKNavigationDelegate {
        let store: AppStore
        var onResize: ((CGFloat) -> Void)?
        var loadedPath: String = ""
        var themePayload: String = "{}"
        private var activeTasks = Set<ObjectIdentifier>()

        init(store: AppStore, onResize: ((CGFloat) -> Void)?) {
            self.store = store
            self.onResize = onResize
        }

        func userContentController(_ userContentController: WKUserContentController, didReceive message: WKScriptMessage) {
            if message.name == "mewResize", let height = message.body as? Double {
                DispatchQueue.main.async { [weak self] in
                    self?.onResize?(CGFloat(height))
                }
            }
        }

        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            webView.evaluateJavaScript("window.postMessage(\(themePayload), '*');", completionHandler: nil)
        }

        func webView(_ webView: WKWebView, start urlSchemeTask: WKURLSchemeTask) {
            let taskId = ObjectIdentifier(urlSchemeTask)
            activeTasks.insert(taskId)
            guard let url = urlSchemeTask.request.url else {
                finish(urlSchemeTask, taskId: taskId, status: 400, mime: "text/plain", data: Data("Bad URL".utf8))
                return
            }
            let path = url.path
            if path == "/artifact-runtime/inline.css" {
                finish(urlSchemeTask, taskId: taskId, status: 200, mime: "text/css; charset=utf-8", data: Data(Self.inlineCSS.utf8))
                return
            }
            if path == "/artifact-runtime/preact.mjs" {
                finish(urlSchemeTask, taskId: taskId, status: 200, mime: "application/javascript; charset=utf-8", data: Data(Self.preactMJS.utf8))
                return
            }
            let comps = path.split(separator: "/", omittingEmptySubsequences: true).map(String.init)
            guard comps.count >= 3, comps[0] == "artifacts" else {
                finish(urlSchemeTask, taskId: taskId, status: 404, mime: "text/plain", data: Data("Not Found".utf8))
                return
            }
            let sessionId = comps[1]
            let artifactId = comps[2]
            let relPath = comps.count > 3 ? comps[3...].joined(separator: "/") : "index.html"
            Task { @MainActor [weak self] in
                guard let self, self.activeTasks.contains(taskId) else { return }
                guard let daemonId = self.store.selectedDaemonId?.nodeId else {
                    self.finish(urlSchemeTask, taskId: taskId, status: 503, mime: "text/plain", data: Data("Disconnected".utf8))
                    return
                }
                do {
                    let result = try await self.store.readArtifactFile(
                        daemonNodeId: daemonId,
                        sessionId: sessionId,
                        artifactId: artifactId,
                        path: relPath
                    )
                    guard self.activeTasks.contains(taskId) else { return }
                    self.finish(urlSchemeTask, taskId: taskId, status: 200, mime: result.mime, data: result.data)
                } catch {
                    guard self.activeTasks.contains(taskId) else { return }
                    self.finish(urlSchemeTask, taskId: taskId, status: 404, mime: "text/plain", data: Data(error.localizedDescription.utf8))
                }
            }
        }

        func webView(_ webView: WKWebView, stop urlSchemeTask: WKURLSchemeTask) {
            activeTasks.remove(ObjectIdentifier(urlSchemeTask))
        }

        private func finish(_ task: WKURLSchemeTask, taskId: ObjectIdentifier, status: Int, mime: String, data: Data) {
            guard activeTasks.remove(taskId) != nil else { return }
            let headers = ["Content-Type": mime, "Content-Length": "\(data.count)", "Cache-Control": "no-cache"]
            if let resp = HTTPURLResponse(url: task.request.url!, statusCode: status, httpVersion: "HTTP/1.1", headerFields: headers) {
                task.didReceive(resp)
            }
            task.didReceive(data)
            task.didFinish()
        }

        private static let preactMJS = #"""
var n,l,u,t,i,r,f,e,o,c,s,a,h={},p=[],v=Array.isArray,y=p.slice,w=Object.assign;function d(n){n&&n.parentNode&&n.remove()}function _(n,l,u){var t,i,r,f={};for(r in l)"key"==r?t=l[r]:"ref"==r&&"function"!=typeof n?i=l[r]:f[r]=l[r];return arguments.length>2&&(f.children=arguments.length>3?y.call(arguments,2):u),g(n,f,t,i,null)}function g(u,t,i,r,f){var e={type:u,props:t,key:i,ref:r,__k:null,__:null,__b:0,__e:null,__c:null,constructor:void 0,__v:null==f?++l:f,__i:-1,__u:0};return null==f&&null!=n.vnode&&n.vnode(e),e}function b(){return{current:null}}function k(n){return n.children}function m(n,l){this.props=n,this.context=l,this.__g=0}function M(n,l){if(null==l)return n.__?M(n.__,n.__i+1):null;for(var u;l<n.__k.length;l++)if(null!=(u=n.__k[l])&&null!=u.__e)return u.__e;return"function"==typeof n.type?M(n):null}function S(n){var l,u;if(null!=(n=n.__)&&null!=n.__c){for(n.__e=null,l=0;l<n.__k.length;l++)if(null!=(u=n.__k[l])&&null!=u.__e){n.__e=u.__e;break}return S(n)}}function $(l){(8&l.__g||!(l.__g|=8)||!t.push(l)||r++)&&i==n.debounceRendering||((i=n.debounceRendering)||queueMicrotask)(x)}function x(){for(var l,u,i,e,o,c,s,a,h=1;t.length;)t.length>h&&t.sort(f),l=t.shift(),h=t.length,8&l.__g&&(i=void 0,o=(e=(u=l).__v).__e,c=[],s=[],(a=u.__P)&&((i=w({},e)).__v=e.__v+1,n.vnode&&n.vnode(i),q(a,i,e,u.__n,a.namespaceURI,32&e.__u?[o]:null,c,null==o?M(e):o,!!(32&e.__u),s,a.ownerDocument),i.__v=e.__v,i.__.__k[i.__i]=i,P(c,i,s),i.__e!=o&&S(i)));r=0}function C(n,l,u,t,i,r,f,e,o,c,s,a){var v,y,w,d,_,g,b,k=t&&t.__k||p,m=l.length;for(o=j(u,l,k,o,m),v=0;v<m;v++)null!=(w=u.__k[v])&&(y=-1==w.__i?h:k[w.__i]||h,w.__i=v,g=q(n,w,y,i,r,f,e,o,c,s,a),d=w.__e,w.ref&&y.ref!=w.ref&&(y.ref&&B(y.ref,null,w),s.push(w.ref,w.__c||d,w)),null==_&&null!=d&&(_=d),(b=!!(4&w.__u))||y.__k===w.__k?o=A(w,o,n,b):"function"==typeof w.type&&void 0!==g?o=g:d&&(o=d.nextSibling),w.__u&=-7);return u.__e=_,o}function j(n,l,u,t,i){var r,f,e,o,c,s=u.length,a=s,h=0;for(n.__k=new Array(i),r=0;r<i;r++)null!=(f=l[r])&&"boolean"!=typeof f&&"function"!=typeof f?(o=r+h,(f=n.__k[r]="string"==typeof f||"number"==typeof f||"bigint"==typeof f||f.constructor==String?g(null,f,null,null,null):v(f)?g(k,{children:f},null,null,null):null==f.constructor&&f.__b>0?g(f.type,f.props,f.key,f.ref?f.ref:null,f.__v):f).__=n,f.__b=n.__b+1,e=null,-1!=(c=f.__i=I(f,u,o,a))&&(a--,(e=u[c])&&(e.__u|=2)),null==e||null==e.__v?(-1==c&&(i>s?h--:i<s&&h++),"function"!=typeof f.type&&(f.__u|=4)):c!=o&&(c==o-1?h--:c==o+1?h++:(c>o?h--:h++,f.__u|=4))):n.__k[r]=null;if(a)for(r=0;r<s;r++)null!=(e=u[r])&&0==(2&e.__u)&&(e.__e==t&&(t=M(e)),D(e,e));return t}function A(n,l,u,t){var i,r;if("function"==typeof n.type){for(i=n.__k,r=0;i&&r<i.length;r++)i[r]&&(i[r].__=n,l=A(i[r],l,u,t));return l}n.__e!=l&&(t&&(l&&n.type&&!l.parentNode&&(l=M(n)),u.insertBefore(n.__e,l||null)),l=n.__e);do{l=l&&l.nextSibling}while(null!=l&&8==l.nodeType);return l}function H(n,l){return l=l||[],null==n||"boolean"==typeof n||(v(n)?n.some(function(n){H(n,l)}):l.push(n)),l}function I(n,l,u,t){var i,r,f,e=n.key,o=n.type,c=l[u],s=null!=c&&0==(2&c.__u);if(null===c&&null==n.key||s&&e==c.key&&o==c.type)return u;if(t>(s?1:0))for(i=u-1,r=u+1;i>=0||r<l.length;)if(null!=(c=l[f=i>=0?i--:r++])&&0==(2&c.__u)&&e==c.key&&o==c.type)return f;return-1}function L(n,l,u){"-"==l[0]?n.setProperty(l,null==u?"":u):n[l]=null==u?"":u}function O(n,l,u,t,i){var r;n:if("style"==l)if("string"==typeof u)n.style.cssText=u;else{if("string"==typeof t&&(n.style.cssText=t=""),t)for(l in t)u&&l in u||L(n.style,l,"");if(u)for(l in u)t&&u[l]==t[l]||L(n.style,l,u[l])}else if("o"==l[0]&&"n"==l[1])r=l!=(l=l.replace(e,"$1")),(l=l.slice(2))[0].toLowerCase()!=l[0]&&(l=l.toLowerCase()),n.__l||(n.__l={}),n.__l[l+r]=u,u?t?u.l=t.l:(u.l=o,n.addEventListener(l,r?s:c,r)):n.removeEventListener(l,r?s:c,r);else{if("http://www.w3.org/2000/svg"==i)l=l.replace(/xlink(H|:h)/,"h").replace(/sName$/,"s");else if("width"!=l&&"height"!=l&&"href"!=l&&"list"!=l&&"form"!=l&&"tabIndex"!=l&&"download"!=l&&"rowSpan"!=l&&"colSpan"!=l&&"role"!=l&&"popover"!=l&&l in n)try{n[l]=null==u?"":u;break n}catch(n){}"function"==typeof u||(null==u||!1===u&&"-"!=l[4]?n.removeAttribute(l):n.setAttribute(l,"popover"==l&&1==u?"":u))}}function T(l){return function(u){if(this.__l){var t=this.__l[u.type+l];if(null==u.u)u.u=o++;else if(u.u<t.l)return;return t(n.event?n.event(u):u)}}}function q(l,u,t,i,r,f,e,o,c,s,a){var h,p,y,_,g,b,M,S,$,x,j,A,H,I,L,O,T,q,P,B,D,F=u.type;if(null!=u.constructor)return null;128&t.__u&&(c=!!(32&t.__u),t.__c.__z&&(o=u.__e=t.__e=(f=t.__c.__z)[0],t.__c.__z=null)),(h=n.__b)&&h(u);n:if("function"==typeof F)try{if(S=u.props,$="prototype"in F&&F.prototype.render,x=(h=F.contextType)&&i[h.__c],j=h?x?x.props.value:h.__:i,t.__c?2&(p=u.__c=t.__c).__g&&(p.__g|=1,M=!0):($?u.__c=p=new F(S,j):(u.__c=p=new m(S,j),p.constructor=F,p.render=E),x&&x.sub(p),p.props=S,p.state||(p.state={}),p.context=j,p.__n=i,y=!0,p.__g|=8,p.__h=[],p._sb=[]),$&&null==p.__s&&(p.__s=p.state),$&&null!=F.getDerivedStateFromProps&&(p.__s==p.state&&(p.__s=w({},p.__s)),w(p.__s,F.getDerivedStateFromProps(S,p.__s))),_=p.props,g=p.state,p.__v=u,y)$&&null==F.getDerivedStateFromProps&&null!=p.componentWillMount&&p.componentWillMount(),$&&null!=p.componentDidMount&&p.__h.push(p.componentDidMount);else{if($&&null==F.getDerivedStateFromProps&&S!==_&&null!=p.componentWillReceiveProps&&p.componentWillReceiveProps(S,j),!(4&p.__g)&&null!=p.shouldComponentUpdate&&!1===p.shouldComponentUpdate(S,p.__s,j)||u.__v==t.__v){for(u.__v!=t.__v&&(p.props=S,p.state=p.__s,p.__g&=-9),u.__e=t.__e,u.__k=t.__k,u.__k.some(function(n){n&&(n.__=u)}),A=0;A<p._sb.length;A++)p.__h.push(p._sb[A]);p._sb=[],p.__h.length&&e.push(p);break n}null!=p.componentWillUpdate&&p.componentWillUpdate(S,p.__s,j),$&&null!=p.componentDidUpdate&&p.__h.push(function(){p.componentDidUpdate(_,g,b)})}if(p.context=j,p.props=S,p.__P=l,p.__g&=-5,H=n.__r,I=0,$){for(p.state=p.__s,p.__g&=-9,H&&H(u),h=p.render(p.props,p.state,p.context),L=0;L<p._sb.length;L++)p.__h.push(p._sb[L]);p._sb=[]}else do{p.__g&=-9,H&&H(u),h=p.render(p.props,p.state,p.context),p.state=p.__s}while(8&p.__g&&++I<25);p.state=p.__s,null!=p.getChildContext&&(i=w({},i,p.getChildContext())),$&&!y&&null!=p.getSnapshotBeforeUpdate&&(b=p.getSnapshotBeforeUpdate(_,g)),O=h,null!=h&&h.type===k&&null==h.key&&(O=V(h.props.children)),o=C(l,v(O)?O:[O],u,t,i,r,f,e,o,c,s,a),u.__u&=-161,p.__h.length&&e.push(p),M&&(p.__g&=-4)}catch(l){if(u.__v=null,c||null!=f)if(l.then){for(T=0,q=!1,u.__u|=c?160:128,u.__c.__z=[],P=0;P<f.length;P++)null==(B=f[P])||q||(8==B.nodeType&&"$s"==B.data?(T>0&&u.__c.__z.push(B),T++,f[P]=null):8==B.nodeType&&"/$s"==B.data?(--T>0&&u.__c.__z.push(B),q=0===T,o=f[P],f[P]=null):T>0&&(u.__c.__z.push(B),f[P]=null));if(!q){for(;o&&8==o.nodeType&&o.nextSibling;)o=o.nextSibling;f[f.indexOf(o)]=null,u.__c.__z=[o]}u.__e=o}else{for(D=f.length;D--;)d(f[D]);N(u)}else u.__e=t.__e,u.__k=t.__k,l.then||N(u);n.__e(l,u,t)}else o=u.__e=z(t.__e,u,t,i,r,f,e,c,s,a);return(h=n.diffed)&&h(u),128&u.__u?void 0:o}function N(n){n&&n.__c&&(n.__c.__g|=4),n&&n.__k&&n.__k.forEach(N)}function P(l,u,t){for(var i=0;i<t.length;i++)B(t[i],t[++i],t[++i]);n.__c&&n.__c(u,l),l.some(function(u){try{l=u.__h,u.__h=[],l.some(function(n){n.call(u)})}catch(l){n.__e(l,u.__v)}})}function V(n){return"object"!=typeof n||null==n||n.__b&&n.__b>0?n:v(n)?n.map(V):w({},n)}function z(l,u,t,i,r,f,e,o,c,s){var a,p,w,_,g,b,k,m,S=t.props,$=u.props,x=u.type;if("svg"==x?r="http://www.w3.org/2000/svg":"math"==x?r="http://www.w3.org/1998/Math/MathML":r||(r="http://www.w3.org/1999/xhtml"),null!=f)for(a=0;a<f.length;a++)if((g=f[a])&&"setAttribute"in g==!!x&&(x?g.localName==x:3==g.nodeType)){l=g,f[a]=null;break}if(null==l){if(null==x)return s.createTextNode($);l=s.createElementNS(r,x,$.is&&$),o&&(n.__m&&n.__m(u,f),o=!1),f=null}if(null==x)S===$||o&&l.data==$||(l.data=$);else{if(f=f&&y.call(l.childNodes),S=t.props||h,!o&&null!=f)for(S={},a=0;a<l.attributes.length;a++)S[(g=l.attributes[a]).name]=g.value;for(a in S)if(g=S[a],"children"==a);else if("dangerouslySetInnerHTML"==a)w=g;else if(!(a in $)){if("value"==a&&"defaultValue"in $||"checked"==a&&"defaultChecked"in $)continue;O(l,a,null,g,r)}for(a in m=1&t.__u,$)g=$[a],"children"==a?_=g:"dangerouslySetInnerHTML"==a?p=g:"value"==a?b=g:"checked"==a?k=g:o&&"function"!=typeof g||S[a]===g&&!m||O(l,a,g,S[a],r);if(p)o||w&&(p.__html==w.__html||p.__html==l.innerHTML)||(l.innerHTML=p.__html),u.__k=[];else if(w&&(l.innerHTML=""),C("template"==x?l.content:l,v(_)?_:[_],u,t,i,"foreignObject"==x?"http://www.w3.org/1999/xhtml":r,f,e,f?f[0]:t.__k&&M(t,0),o,c,s),null!=f)for(a=f.length;a--;)d(f[a]);o||(a="value","progress"==x&&null==b?l.removeAttribute("value"):null==b||b===l[a]&&("progress"!==x||b)||O(l,a,b,S[a],r),a="checked",null!=k&&k!=l[a]&&O(l,a,k,S[a],r))}return l}function B(l,u,t){try{if("function"==typeof l){var i="function"==typeof l.__u;i&&l.__u(),i&&null==u||(l.__u=l(u))}else l.current=u}catch(l){n.__e(l,t)}}function D(l,u,t){var i,r;if(n.unmount&&n.unmount(l),(i=l.ref)&&(i.current&&i.current!=l.__e||B(i,null,u)),null!=(i=l.__c)){if(i.componentWillUnmount)try{i.componentWillUnmount()}catch(l){n.__e(l,u)}i.__P=null}if(i=l.__k)for(r=0;r<i.length;r++)i[r]&&D(i[r],u,t||"function"!=typeof l.type);t||d(l.__e),l.__e&&l.__e.__l&&(l.__e.__l=null),l.__e=l.__c=l.__=null}function E(n,l,u){return this.constructor(n,u)}function F(l,u){var t,i,r,f;u==document&&(u=document.documentElement),n.__&&n.__(l,u),i=(t=!!(l&&32&l.__u))?null:u.__k,l=u.__k=_(k,null,[l]),r=[],f=[],q(u,l,i||h,h,u.namespaceURI,i?null:u.firstChild?y.call(u.childNodes):null,r,i?i.__e:u.firstChild,t,f,u.ownerDocument),P(r,l,f)}function G(n,l){n.__u|=32,F(n,l)}function J(n,l,u){var t,i,r,f=w({},n.props);for(r in l)"key"==r?t=l[r]:"ref"==r&&"function"!=typeof n.type?i=l[r]:f[r]=l[r];return arguments.length>2&&(f.children=arguments.length>3?y.call(arguments,2):u),g(n.type,f,t||n.key,i||n.ref,null)}function K(n){function l(n){var u,t;return this.getChildContext||(u=new Set,(t={})[l.__c]=this,this.getChildContext=function(){return t},this.componentWillUnmount=function(){u=null},this.shouldComponentUpdate=function(n){this.props.value!=n.value&&u.forEach(function(n){n.__g|=4,$(n)})},this.sub=function(n){u.add(n);var l=n.componentWillUnmount;n.componentWillUnmount=function(){u&&u.delete(n),l&&l.call(n)}}),n.children}return l.__c="__cC"+a++,l.__=n,l.Provider=l.__l=(l.Consumer=function(n,l){return n.children(l)}).contextType=l,l}n={__e:function(n,l,u,t){for(var i,f,e;l=l.__;)if((i=l.__c)&&!(1&i.__g)){i.__g|=4;try{if((f=i.constructor)&&null!=f.getDerivedStateFromError&&(i.setState(f.getDerivedStateFromError(n)),e=8&i.__g),null!=i.componentDidCatch&&(i.componentDidCatch(n,t||{}),e=8&i.__g),e)return void(i.__g|=2)}catch(l){n=l}}throw r=0,n}},l=0,u=function(n){return null!=n&&null==n.constructor},m.prototype.setState=function(n,l){var u;u=null!=this.__s&&this.__s!=this.state?this.__s:this.__s=w({},this.state),"function"==typeof n&&(n=n(w({},u),this.props)),n&&w(u,n),null!=n&&this.__v&&(l&&this._sb.push(l),$(this))},m.prototype.forceUpdate=function(n){this.__v&&(this.__g|=4,n&&this.__h.push(n),$(this))},m.prototype.render=k,t=[],r=0,f=function(n,l){return n.__v.__b-l.__v.__b},e=/(PointerCapture)$|Capture$/i,o=0,c=T(!1),s=T(!0),a=0;export{m as Component,k as Fragment,J as cloneElement,K as createContext,_ as createElement,b as createRef,_ as h,G as hydrate,u as isValidElement,n as options,F as render,H as toChildArray};
//# sourceMappingURL=preact.mjs.map

"""#

        private static let inlineCSS = """
        :root { color-scheme: light dark; --background: #fff; --foreground: #18181b; --card: #f4f4f5; --muted: #f4f4f5; --muted-foreground: #71717a; --border: #e4e4e7; --input: #e4e4e7; --primary: #2563eb; --primary-foreground: #fff; --chart-1: #16a34a; --chart-2: #2563eb; --chart-3: #d97706; --chart-4: #9333ea; }
        *, *::before, *::after { box-sizing: border-box; border-color: var(--border); }
        html, body { margin: 0; padding: 0; background: var(--background); color: var(--foreground); font-family: -apple-system, sans-serif; font-size: 14px; line-height: 1.5; }
        .mew-inline-root { padding: 12px; }
        .bg-background { background-color: var(--background); } .text-foreground { color: var(--foreground); }
        .bg-card { background-color: var(--card); } .bg-muted { background-color: var(--muted); }
        .text-muted-foreground { color: var(--muted-foreground); } .bg-primary { background-color: var(--primary); }
        .text-primary-foreground { color: var(--primary-foreground); } .border-border { border-color: var(--border); }
        .flex { display: flex; } .grid { display: grid; } .flex-col { flex-direction: column; } .flex-wrap { flex-wrap: wrap; } .flex-1 { flex: 1; }
        .items-center { align-items: center; } .justify-between { justify-content: space-between; }
        .gap-1 { gap: 4px; } .gap-2 { gap: 8px; } .gap-3 { gap: 12px; } .gap-4 { gap: 16px; }
        .p-2 { padding: 8px; } .p-3 { padding: 12px; } .p-4 { padding: 16px; } .px-3 { padding-left: 12px; padding-right: 12px; } .py-1 { padding-top: 4px; padding-bottom: 4px; } .py-2 { padding-top: 8px; padding-bottom: 8px; }
        .w-full { width: 100%; } .border { border: 1px solid var(--border); } .rounded-md { border-radius: 6px; } .rounded-lg { border-radius: 8px; }
        .text-xs { font-size: 12px; } .text-sm { font-size: 14px; } .text-lg { font-size: 18px; } .font-medium { font-weight: 500; } .font-semibold { font-weight: 600; }
        button, input, select { font: inherit; color: inherit; }
        input[type="text"], input[type="number"], select { background: var(--background); border: 1px solid var(--input); border-radius: 6px; padding: 6px 10px; }
        button { border: 1px solid var(--border); border-radius: 6px; background: var(--card); padding: 6px 12px; }
        button.bg-primary { background: var(--primary); color: var(--primary-foreground); border-color: var(--primary); }
        """
    }
}

private extension Color {
    var hexString: String? {
        let ui = UIColor(self)
        var r: CGFloat = 0, g: CGFloat = 0, b: CGFloat = 0, a: CGFloat = 0
        guard ui.getRed(&r, green: &g, blue: &b, alpha: &a) else { return nil }
        return String(format: "#%02x%02x%02x", Int(round(r * 255)), Int(round(g * 255)), Int(round(b * 255)))
    }
}
