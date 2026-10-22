import SwiftUI
import MewMobileCore

struct ReviewView: View {
    let daemon: String
    let sessionId: String

    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss

    private var review: WorkspaceReview? {
        guard store.review?.daemon == daemon, store.review?.sessionId == sessionId else { return nil }
        return store.review
    }

    var body: some View {
        NavigationStack {
            Group {
                if let review {
                    if review.isLoading {
                        ProgressView("Loading changes…")
                    } else if let error = review.error {
                        ContentUnavailableView {
                            Label("Couldn't load changes", systemImage: "exclamationmark.triangle")
                        } description: {
                            Text(error)
                        } actions: {
                            Button("Try again", action: refresh)
                        }
                    } else if review.files.isEmpty {
                        ContentUnavailableView("No changes", systemImage: "checkmark.circle",
                            description: Text("This workspace matches its latest commit."))
                    } else {
                        fileList(review.files)
                    }
                } else {
                    ContentUnavailableView("Review changes", systemImage: "doc.text.magnifyingglass",
                        description: Text("Refresh to load this session's workspace changes."))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .navigationTitle("Changes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Refresh", systemImage: "arrow.clockwise", action: refresh)
                }
            }
            .task { refresh() }
        }
    }

    private func refresh() {
        store.refreshReview(daemon: daemon, sessionId: sessionId)
    }

    private func fileList(_ files: [ReviewFile]) -> some View {
        List {
            Section {
                ForEach(files, id: \.path) { file in
                    NavigationLink {
                        FileReviewView(file: file)
                    } label: {
                        VStack(alignment: .leading, spacing: 5) {
                            Text(file.path)
                                .font(.subheadline.monospaced())
                                .foregroundStyle(.primary)
                            if let oldPath = file.oldPath, oldPath != file.path {
                                Text("From \(oldPath)")
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                            HStack(spacing: 10) {
                                Text(file.status.label)
                                    .foregroundStyle(.secondary)
                                if file.added > 0 {
                                    Text("+\(file.added)").foregroundStyle(.green)
                                }
                                if file.removed > 0 {
                                    Text("−\(file.removed)").foregroundStyle(.red)
                                }
                            }
                            .font(.caption.monospaced())
                        }
                        .padding(.vertical, 4)
                    }
                }
            } header: {
                Text("\(files.count) changed \(files.count == 1 ? "file" : "files")")
            } footer: {
                Text("Compared with the latest commit. Includes untracked files.")
            }
        }
    }
}

private struct FileReviewView: View {
    let file: ReviewFile

    var body: some View {
        Group {
            if file.status == .binary {
                ContentUnavailableView("Binary file", systemImage: "doc",
                    description: Text("Text changes aren't available for this file."))
            } else if file.hunks.isEmpty {
                ContentUnavailableView(file.status.label, systemImage: "doc",
                    description: Text("No text changes to display."))
            } else {
                ScrollView([.horizontal, .vertical]) {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(file.hunks.indices, id: \.self) { index in
                            let hunk = file.hunks[index]
                            Text("@@ −\(hunk.oldStart),\(hunk.oldCount) +\(hunk.newStart),\(hunk.newCount) @@")
                                .foregroundStyle(.secondary)
                                .padding(8)
                                .frame(maxWidth: .infinity, alignment: .leading)
                                .background(Color(.secondarySystemBackground))
                            ForEach(hunk.lines.indices, id: \.self) { lineIndex in
                                ReviewLineView(line: hunk.lines[lineIndex])
                            }
                        }
                    }
                    .font(.caption.monospaced())
                    .padding(.vertical, 8)
                }
                .defaultScrollAnchor(.topLeading)
                .textSelection(.enabled)
            }
        }
        .navigationTitle(file.path.components(separatedBy: "/").last ?? file.path)
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct ReviewLineView: View {
    let line: ReviewLine

    private var color: Color {
        switch line.kind {
        case .addition: return .green
        case .deletion: return .red
        case .context: return .secondary
        }
    }

    private var marker: String {
        switch line.kind {
        case .addition: return "+"
        case .deletion: return "−"
        case .context: return " "
        }
    }

    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Text(line.oldLine.map(String.init) ?? "")
                .foregroundStyle(.secondary)
                .frame(width: 42, alignment: .trailing)
            Text(line.newLine.map(String.init) ?? "")
                .foregroundStyle(.secondary)
                .frame(width: 42, alignment: .trailing)
            Text(marker)
                .foregroundStyle(color)
                .frame(width: 10)
            Text(line.text.hasSuffix("\n") ? String(line.text.dropLast()) : line.text)
                .fixedSize(horizontal: true, vertical: false)
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 2)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(line.kind == .context ? Color.clear : color.opacity(0.12))
        .accessibilityElement(children: .combine)
    }
}

private extension ReviewFileStatus {
    var label: String {
        switch self {
        case .added: return "Added"
        case .modified: return "Modified"
        case .deleted: return "Deleted"
        case .renamed: return "Renamed"
        case .unchanged: return "Unchanged"
        case .binary: return "Binary"
        }
    }
}
