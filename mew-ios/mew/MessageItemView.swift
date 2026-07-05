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
/// - `.toolCall`: compact row with a state indicator, expandable to show input
/// - `.error`: red text with a warning icon
struct MessageItemView: View {
    let message: ChatMessage

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
                        .font(.body)
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
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .frame(maxWidth: 320, alignment: .trailing)
    }

    // MARK: Assistant

    @ViewBuilder
    private var assistantContent: some View {
        VStack(alignment: .leading, spacing: 8) {
            ForEach(message.parts, id: \.id) { part in
                partView(part)
            }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .frame(maxWidth: 360, alignment: .leading)
        // Assistant messages render with no background — the chat surface
        // shows through and user bubbles provide the only visual separation.
    }

    @ViewBuilder
    private func partView(_ part: MessagePart) -> some View {
        switch part.kind {
        case .text:
            if let text = part.text, !text.isEmpty {
                MarkdownView(text: text, config: .mew)
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
                Text(markdown(text))
                    .font(.footnote)
                    .foregroundStyle(.secondary)
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
        ToolCallRow(part: part)
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

// MARK: - Tool call row

/// Compact, expandable row representing a `.toolCall` part.
private struct ToolCallRow: View {
    let part: MessagePart

    @State private var expanded = false

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
        VStack(alignment: .leading, spacing: 6) {
            HStack(spacing: 8) {
                Image(systemName: "wrench.and.screwdriver")
                    .font(.subheadline)
                    .foregroundStyle(.secondary)

                Text(part.toolName ?? "tool")
                    .font(.subheadline.weight(.medium))
                    .lineLimit(1)

                Spacer(minLength: 4)

                stateIndicator

                if let duration = formattedDuration {
                    Text(duration)
                        .font(.caption2.monospaced())
                        .foregroundStyle(.tertiary)
                }
            }
            .contentShape(Rectangle())
            .onTapGesture { withAnimation(.easeInOut(duration: 0.2)) { expanded.toggle() } }

            if expanded {
                detailContent
            }
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
                    Text(prettyJSON(input))
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

    private var formattedDuration: String? {
        guard let start = part.toolTimeStart else { return nil }
        let end = part.toolTimeEnd ?? Int64(Date().timeIntervalSince1970)
        let elapsed = end - start
        if elapsed < 1 { return nil }
        if elapsed < 60 { return "\(elapsed)s" }
        return "\(elapsed / 60)m\(elapsed % 60)s"
    }

    private func prettyJSON(_ raw: String) -> String {
        guard let data = raw.data(using: .utf8),
              let json = try? JSONSerialization.jsonObject(with: data),
              let pretty = try? JSONSerialization.data(withJSONObject: json, options: [.prettyPrinted, .sortedKeys]),
              let result = String(data: pretty, encoding: .utf8) else {
            return raw
        }
        return result
    }

    @ViewBuilder
    private var stateIndicator: some View {
        switch state {
        case .pending:
            Image(systemName: "circle")
                .foregroundStyle(.secondary)
                .font(.caption)
        case .running:
            ProgressView()
                .controlSize(.small)
                .tint(.blue)
        case .completed:
            Image(systemName: "checkmark.circle.fill")
                .foregroundStyle(.green)
                .font(.caption)
        case .error:
            Image(systemName: "xmark.circle.fill")
                .foregroundStyle(.red)
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
    @StateObject private var source = LiveMarkdownSource()

    var body: some View {
        HStack {
            VStack(alignment: .leading, spacing: 6) {
                if text.isEmpty {
                    HStack(spacing: 4) {
                        TypingDot()
                        TypingDot(delay: 0.2)
                        TypingDot(delay: 0.4)
                    }
                } else {
                    StreamedMarkdownView(source: source, config: .mew)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .frame(maxWidth: 360, alignment: .leading)
            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 16, style: .continuous))

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
    var delay: Double = 0
    @State private var animate = false

    var body: some View {
        Circle()
            .fill(Color.secondary)
            .frame(width: 6, height: 6)
            .opacity(animate ? 0.3 : 1.0)
            .animation(.easeInOut(duration: 0.6).repeatForever(autoreverses: true).delay(delay), value: animate)
            .onAppear { animate = true }
    }
}
