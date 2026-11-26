import SwiftUI
import MewMobileCore

struct ReviewView: View {
    let daemon: String
    let sessionId: String

    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme

    private var review: WorkspaceReview? {
        guard store.review?.daemon == daemon, store.review?.sessionId == sessionId else { return nil }
        return store.review
    }

    var body: some View {
        NavigationStack {
            Group {
                if let review {
                    if !review.files.isEmpty {
                        fileList(review.files)
                    } else if review.isLoading {
                        ProgressView("Loading changes…")
                    } else if let error = review.error {
                        ContentUnavailableView {
                            Label("Couldn't load changes", systemImage: "exclamationmark.triangle")
                        } description: {
                            Text(error)
                        } actions: {
                            Button("Try again", action: refresh)
                        }
                    } else {
                        ContentUnavailableView("No changes", systemImage: "checkmark.circle",
                            description: Text("This workspace matches its latest commit."))
                    }
                } else {
                    ContentUnavailableView("Review changes", systemImage: "doc.text.magnifyingglass",
                        description: Text("Refresh to load this session's workspace changes."))
                }
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .background(theme.background.ignoresSafeArea())
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if let review, !review.files.isEmpty {
                    RefreshStatusView(isLoading: review.isLoading, error: review.error, retry: refresh)
                }
            }
            .navigationTitle("Changes")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Refresh", systemImage: "arrow.clockwise", action: refresh)
                        .disabled(review?.isLoading == true)
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
                                .font(.mewMono(15))
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
                                    Text("+\(file.added)")
                                        .foregroundStyle(.green)
                                        .monospacedDigit()
                                }
                                if file.removed > 0 {
                                    Text("−\(file.removed)")
                                        .foregroundStyle(.red)
                                        .monospacedDigit()
                                }
                            }
                            .font(.mewMono(12))
                        }
                        .padding(.vertical, 4)
                    }
                }
            } header: {
                Text("\(files.count) changed \(files.count == 1 ? "file" : "files")")
            } footer: {
                Text("Compared with the latest commit. Includes untracked files.")
            }
            .listRowBackground(theme.card)
        }
        .mewThemedForm(theme)
    }
}

private struct FileReviewView: View {
    let file: ReviewFile
    @EnvironmentObject private var store: AppStore
    @Environment(\.mewTheme) private var theme

    var body: some View {
        Group {
            if file.status == .binary {
                ContentUnavailableView("Binary file", systemImage: "doc",
                    description: Text("Text changes aren't available for this file."))
            } else if file.hunks.isEmpty {
                ContentUnavailableView(file.status.label, systemImage: "doc",
                    description: Text("No text changes to display."))
            } else {
                ReviewDiffView(file: file, settings: store.phoneSettings)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(theme.background.ignoresSafeArea())
        .navigationTitle(file.path.components(separatedBy: "/").last ?? file.path)
        .navigationBarTitleDisplayMode(.inline)
    }
}

struct ReviewDiffView: View {
    let file: ReviewFile
    let settings: PhoneSettings
    @ScaledMetric(relativeTo: .caption) private var baseTextSize = 13

    private var fontSize: CGFloat { baseTextSize * settings.reviewTextSize.scale }
    private var gutterWidth: CGFloat {
        let lastLine = file.hunks.lazy.flatMap(\.lines).reduce(UInt64(0)) { max($0, $1.oldLine ?? 0, $1.newLine ?? 0) }
        return CGFloat(max(2, String(lastLine).count)) * fontSize * 0.65
    }

    var body: some View {
        ScrollView(settings.reviewWrapLines ? .vertical : [.horizontal, .vertical]) {
            LazyVStack(alignment: .leading, spacing: 0) {
                ForEach(file.hunks.indices, id: \.self) { index in
                    let hunk = file.hunks[index]
                    Text("@@ −\(hunk.oldStart),\(hunk.oldCount) +\(hunk.newStart),\(hunk.newCount) @@")
                        .foregroundStyle(.secondary)
                        .padding(8)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(Color(.secondarySystemBackground))
                    ForEach(hunk.lines.indices, id: \.self) { lineIndex in
                        ReviewLineView(line: hunk.lines[lineIndex], wrapLines: settings.reviewWrapLines,
                            showLineNumbers: settings.reviewLineNumbers, fontSize: fontSize, gutterWidth: gutterWidth)
                    }
                }
            }
            .font(.mewMono(fontSize))
            .padding(.vertical, 8)
        }
        .defaultScrollAnchor(.topLeading)
        .textSelection(.enabled)
    }
}

struct ReviewLineView: View {
    let line: ReviewLine
    let wrapLines: Bool
    let showLineNumbers: Bool
    let fontSize: CGFloat
    let gutterWidth: CGFloat

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
            if showLineNumbers {
                Text(line.oldLine.map(String.init) ?? "")
                    .foregroundStyle(.secondary)
                    .frame(width: gutterWidth, alignment: .trailing)
                Text(line.newLine.map(String.init) ?? "")
                    .foregroundStyle(.secondary)
                    .frame(width: gutterWidth, alignment: .trailing)
            }
            Text(marker)
                .foregroundStyle(color)
                .frame(width: fontSize * 0.65)
            Text(line.text.hasSuffix("\n") ? String(line.text.dropLast()) : line.text)
                .fixedSize(horizontal: !wrapLines, vertical: true)
                .frame(maxWidth: wrapLines ? .infinity : nil, alignment: .leading)
        }
        .font(.mewMono(fontSize))
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
