import SwiftUI
import MewMobileCore

/// A compact collapsible checklist that appears above the composer when the
/// agent has todo items. Renders each `TodoItem` with a status-appropriate
/// indicator (checkbox, spinner, strikethrough) and dims items whose
/// `depends_on` prerequisites are not yet done.
struct TodoPanelView: View {
    let todos: [TodoItem]

    @State private var isExpanded: Bool = true

    var body: some View {
        if !todos.isEmpty {
            VStack(spacing: 0) {
                Button {
                    withAnimation(.easeInOut(duration: 0.2)) {
                        isExpanded.toggle()
                    }
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: isExpanded ? "chevron.down" : "chevron.right")
                            .font(.caption2)
                            .foregroundStyle(.secondary)
                        Text("Tasks")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text("\(completedCount)/\(todos.count)")
                            .font(.caption2)
                            .foregroundStyle(.tertiary)
                        Spacer()
                    }
                    .padding(.horizontal, 16)
                    .padding(.vertical, 8)
                }
                .buttonStyle(.plain)

                if isExpanded {
                    VStack(spacing: 0) {
                        ForEach(todos, id: \.id) { todo in
                            todoRow(todo)
                        }
                    }
                    .padding(.bottom, 4)
                }
            }
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(Color(.secondarySystemBackground).opacity(0.7))
            )
            .padding(.horizontal, 12)
            .padding(.bottom, 4)
        }
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
        .padding(.horizontal, 16)
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
