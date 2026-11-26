import SwiftUI
import MewMobileCore

/// A compact collapsible checklist that appears above the transcript when the
/// agent has todo items. Defaults to collapsed to avoid obscuring messages,
/// uses a translucent material surface so scrolling text does not bleed
/// through, and animates with a critically damped spring.
struct TodoPanelView: View {
    let todos: [TodoItem]

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @State private var isExpanded: Bool = false

    private var activeTask: TodoItem? {
        todos.first { $0.status == "in_progress" }
    }

    private var allDone: Bool {
        !todos.isEmpty && completedCount == todos.count
    }

    var body: some View {
        if !todos.isEmpty {
            let shape = RoundedRectangle(cornerRadius: Theme.Layout.controlRadius, style: .continuous)
            VStack(spacing: 0) {
                Button {
                    withAnimation(Theme.Motion.value(.spring(response: 0.3, dampingFraction: 1.0), reduced: reduceMotion)) {
                        isExpanded.toggle()
                    }
                } label: {
                    HStack(spacing: 8) {
                        headerIcon
                        Text(headerTitle)
                            .font(.caption.weight(activeTask != nil && !isExpanded ? .medium : .semibold))
                            .foregroundStyle(activeTask != nil && !isExpanded ? .primary : .secondary)
                            .lineLimit(1)
                            .truncationMode(.tail)
                        Spacer(minLength: 8)
                        Text("\(completedCount)/\(todos.count)")
                            .font(.caption2.monospacedDigit())
                            .foregroundStyle(.secondary)
                        Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.tertiary)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .contentShape(Rectangle())
                }
                .buttonStyle(MewPressButtonStyle())

                if isExpanded {
                    Divider()
                        .opacity(0.4)
                        .padding(.horizontal, 12)
                    VStack(spacing: 0) {
                        ForEach(todos, id: \.id) { todo in
                            todoRow(todo)
                        }
                    }
                    .padding(.vertical, 6)
                }
            }
            .background {
                if reduceTransparency {
                    shape.fill(Color(.secondarySystemBackground))
                } else {
                    shape.fill(.regularMaterial)
                }
            }
            .overlay(
                shape.strokeBorder(Color(.separator).opacity(0.45), lineWidth: 0.5)
            )
            .shadow(color: .black.opacity(0.08), radius: 6, y: 2)
            .padding(.horizontal, 12)
            .padding(.vertical, 4)
        }
    }

    // MARK: - Header

    @ViewBuilder
    private var headerIcon: some View {
        if activeTask != nil {
            ProgressView()
                .controlSize(.mini)
                .frame(width: 14, height: 14)
        } else if allDone {
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 13))
                .foregroundStyle(.green)
        } else {
            Image(systemName: "checklist")
                .font(.system(size: 13))
                .foregroundStyle(.secondary)
        }
    }

    private var headerTitle: String {
        if !isExpanded, let activeTask {
            return activeTask.content
        }
        return "Tasks"
    }

    // MARK: - Row

    @ViewBuilder
    private func todoRow(_ todo: TodoItem) -> some View {
        let isDimmed = !dependenciesMet(todo)
        HStack(spacing: 8) {
            statusIcon(todo.status, isDimmed: isDimmed)

            Text(todo.content)
                .font(.caption)
                .strikethrough(todo.status == "done", color: .secondary)
                .foregroundStyle(isDimmed ? .tertiary : .primary)
                .lineLimit(2)

            Spacer(minLength: 0)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 4)
    }

    // MARK: - Status icon

    @ViewBuilder
    private func statusIcon(_ status: String, isDimmed: Bool) -> some View {
        switch status {
        case "done":
            Image(systemName: "checkmark.circle.fill")
                .font(.system(size: 14))
                .foregroundStyle(.green)
        case "in_progress":
            ProgressView()
                .controlSize(.mini)
                .frame(width: 14, height: 14)
        case "blocked":
            Image(systemName: "hand.raised.fill")
                .font(.system(size: 12))
                .foregroundStyle(.orange)
        default: // pending
            Image(systemName: "circle")
                .font(.system(size: 14))
                .foregroundStyle(isDimmed ? .tertiary : .secondary)
        }
    }

    // MARK: - Dependency check

    /// Returns true if all items in `todo.depends_on` have status "done".
    private func dependenciesMet(_ todo: TodoItem) -> Bool {
        guard !todo.dependsOn.isEmpty else { return true }
        let doneIds = Set(todos.filter { $0.status == "done" }.map { $0.id })
        return todo.dependsOn.allSatisfy { doneIds.contains($0) }
    }

    // MARK: - Computed

    private var completedCount: Int {
        todos.filter { $0.status == "done" }.count
    }
}
