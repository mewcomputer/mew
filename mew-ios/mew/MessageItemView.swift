import SwiftUI
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
    var fontChoice: MewFontChoice = .miSans
    @Binding var disclosures: ToolDisclosures
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var isUser: Bool { message.role == "user" }

    var body: some View {
        HStack {
            if isUser {
                Spacer(minLength: 0)
                userBubble
            } else {
                assistantContent
                Spacer(minLength: 0)
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 3)
    }

    // MARK: User

    @ViewBuilder
    private var userBubble: some View {
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
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous))
        .frame(maxWidth: 320, alignment: .trailing)
    }

    // MARK: Assistant

    @ViewBuilder
    private var assistantContent: some View {
        VStack(alignment: .leading, spacing: 4) {
            if !message.parts.isEmpty && message.parts.allSatisfy({ $0.kind == .toolCall }) {
                ToolGroupRow(parts: message.parts, disclosures: $disclosures)
            } else {
                ForEach(message.parts, id: \.id) { part in
                    partView(part)
                }
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, message.parts.allSatisfy { $0.kind == .toolCall } ? 0 : 10)
        .frame(maxWidth: 360, alignment: .leading)
        // Assistant messages render with no background — the chat surface
        // shows through and user bubbles provide the only visual separation.
    }

    @ViewBuilder
    private func partView(_ part: MessagePart) -> some View {
        switch part.kind {
        case .text:
            if let text = part.text, !text.isEmpty {
                MarkdownView(text: text, config: .mewStatic(fontChoice: fontChoice))
            }
        case .reasoning:
            reasoningView(part)
        case .toolCall:
            toolCallView(part)
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
    private func reasoningView(_ part: MessagePart) -> some View {
        let text = part.text ?? ""
        DisclosureGroup {
            if !text.isEmpty {
                MarkdownView(text: text, config: .mewReasoning(fontChoice: fontChoice))
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

// MARK: - Tool group row

/// Consecutive activity at one conversation level, with individual calls inside.
private struct ToolGroupRow: View {
    let parts: [MessagePart]
    @Binding var disclosures: ToolDisclosures

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var activityCache = ToolActivity.Cache()
    private var expanded: Bool { disclosures.isGroupExpanded(parts) }

    private var anyActive: Bool {
        parts.contains { $0.toolState != "completed" && $0.toolState != "error" }
    }
    private var anyError: Bool { parts.contains { $0.toolState == "error" } }

    var body: some View {
        let summary = ToolActivity.summary(parts, cache: activityCache)
        VStack(alignment: .leading, spacing: 4) {
            Button {
                withAnimation(Theme.Motion.value(Theme.Motion.disclosure, reduced: reduceMotion)) {
                    disclosures.setGroupExpanded(!expanded, parts: parts)
                }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: ToolActivity.symbol(parts[0]))
                        .font(.subheadline)
                    Text(summary)
                        .font(.subheadline)
                        .lineLimit(2)
                        .multilineTextAlignment(.leading)
                    Spacer(minLength: 4)
                    aggregateIndicator
                    Image(systemName: expanded ? "chevron.down" : "chevron.right")
                        .font(.caption2)
                }
                .foregroundStyle(.secondary)
                .frame(minHeight: 44)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityLabel(summary)
            .accessibilityValue(ToolActivity.status(parts))
            .accessibilityHint(expanded ? "Hide tool calls" : "Show tool calls")

            if expanded {
                VStack(alignment: .leading, spacing: 4) {
                    ForEach(parts, id: \.id) { part in
                        ToolCallRow(part: part, expandedCalls: $disclosures.expandedCalls)
                    }
                }
                .padding(.leading, 16)
            }
        }
    }

    @ViewBuilder
    private var aggregateIndicator: some View {
        HStack(spacing: 6) {
            if anyActive {
                ProgressView().controlSize(.small).tint(Theme.toolRunning)
            }
            if anyError {
                Image(systemName: "xmark.circle.fill").foregroundStyle(Theme.toolError)
            } else if !anyActive {
                Image(systemName: "checkmark.circle.fill").foregroundStyle(Theme.toolCompleted)
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
                    }
                }
            }
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
                VStack(alignment: .leading, spacing: 4) {
                    Text("Input")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Text(FormatHelper.prettyJSON(input))
                        .font(.caption.monospaced())
                        .foregroundStyle(.primary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }

            // Output (completed or running)
            if let output = part.toolOutput, !output.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Output")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.secondary)
                    Text(output)
                        .font(.caption.monospaced())
                        .foregroundStyle(.primary)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .padding(8)
                        .background(Color(.tertiarySystemBackground), in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                }
            }

            // Error
            if let error = part.toolError, !error.isEmpty {
                VStack(alignment: .leading, spacing: 4) {
                    Text("Error")
                        .font(.caption.weight(.semibold))
                        .foregroundStyle(.red)
                    Text(error)
                        .font(.caption.monospaced())
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
                .foregroundStyle(Theme.toolPending)
                .font(.caption)
        case .running:
            ProgressView()
                .controlSize(.small)
                .tint(Theme.toolRunning)
        case .completed:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(Theme.toolCompleted)
                .font(.caption)
        case .error:
            Image(systemName: "xmark.circle.fill")
                .foregroundStyle(Theme.toolError)
                .font(.caption)
        }
    }

    private enum ToolState { case pending, running, completed, error }
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
    var fontChoice: MewFontChoice = .miSans
    @StateObject private var source = LiveMarkdownSource()

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 6) {
                if text.isEmpty {
                    HStack(spacing: 4) {
                        TypingDot()
                    }
                } else {
                    StreamedMarkdownView(source: source, config: .mew(fontChoice: fontChoice))
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .frame(maxWidth: 360, alignment: .leading)
            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: Theme.Layout.panelRadius, style: .continuous))

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 3)
        .onChange(of: text, initial: true) { _, newValue in
            source.yield(newValue)
        }
    }
}

/// A single pulsing dot used in the "assistant is typing" indicator.
private struct TypingDot: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var animate = false

    var body: some View {
        Circle()
            .fill(Color.secondary)
            .frame(width: 6, height: 6)
            .opacity(animate ? 0.3 : 1.0)
            .animation(
                reduceMotion ? nil : .easeInOut(duration: 0.6).repeatForever(autoreverses: true),
                value: animate
            )
            .onAppear { if !reduceMotion { animate = true } }
    }
}
