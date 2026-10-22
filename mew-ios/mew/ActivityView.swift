import SwiftUI
import MewMobileCore

private enum ActivityDestination: Hashable {
    case worker(id: String, name: String)
    case job(id: String, command: String)
}

struct ActivityView: View {
    let daemon: String
    let sessionId: String
    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Group {
                if store.subagents.isEmpty && store.jobs.isEmpty {
                    ContentUnavailableView("No workers or jobs", systemImage: "person.2",
                        description: Text("Active workers and background commands appear here."))
                } else {
                    List {
                        if !store.subagents.isEmpty {
                            Section("Workers") {
                                ForEach(store.subagents, id: \.taskId) { worker in
                                    let name = worker.displayName ?? worker.name
                                    if let child = worker.childSessionId {
                                        NavigationLink(value: ActivityDestination.worker(id: child, name: name)) {
                                            workerLabel(worker, name: name)
                                        }
                                    } else {
                                        workerLabel(worker, name: name)
                                    }
                                }
                            }
                        }
                        if !store.jobs.isEmpty {
                            Section("Background jobs") {
                                ForEach(store.jobs, id: \.jobId) { job in
                                    NavigationLink(value: ActivityDestination.job(id: job.jobId, command: job.command)) {
                                        VStack(alignment: .leading, spacing: 5) {
                                            Text(job.command).font(.subheadline.monospaced()).lineLimit(2)
                                            Text(job.state.capitalized).font(.caption).foregroundStyle(.secondary)
                                        }
                                        .padding(.vertical, 4)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            .navigationTitle("Workers & jobs")
            .navigationBarTitleDisplayMode(.inline)
            .navigationDestination(for: ActivityDestination.self) { destination in
                switch destination {
                case .worker(let id, let name):
                    WorkerHistoryView(daemon: daemon, sessionId: sessionId, childSessionId: id, name: name)
                case .job(let id, let command):
                    JobOutputView(daemon: daemon, sessionId: sessionId, jobId: id, command: command)
                }
            }
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") { dismiss() }
                }
            }
        }
    }

    private func workerLabel(_ worker: WorkerInfo, name: String) -> some View {
        VStack(alignment: .leading, spacing: 5) {
            Text(name.isEmpty ? "Worker" : name).font(.headline)
            Text(worker.status).font(.subheadline).foregroundStyle(.secondary)
            if let tool = worker.toolName {
                Label(tool, systemImage: "wrench.and.screwdriver").font(.caption).foregroundStyle(.secondary)
            }
        }
        .padding(.vertical, 4)
    }
}

private struct WorkerHistoryView: View {
    let daemon: String
    let sessionId: String
    let childSessionId: String
    let name: String
    @State private var toolDisclosures = ToolDisclosures()
    @EnvironmentObject private var store: AppStore

    var body: some View {
        Group {
            if let history = store.workerHistory, history.daemon == daemon,
               history.sessionId == sessionId, history.childSessionId == childSessionId {
                if !history.messages.isEmpty {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 4) {
                            ForEach(ConversationItem.grouped(messages: history.messages)) { item in
                                MessageItemView(message: item.message, fontChoice: store.fontChoice, disclosures: $toolDisclosures)
                            }
                        }
                    }
                } else if history.isLoading {
                    ProgressView("Loading worker history…")
                } else if let error = history.error {
                    ContentUnavailableView {
                        Label("Couldn't load worker history", systemImage: "exclamationmark.triangle")
                    } description: {
                        Text(error)
                    } actions: {
                        Button("Try again", action: refresh)
                    }
                } else {
                    ContentUnavailableView("No messages yet", systemImage: "text.bubble",
                        description: Text("Refresh to read the worker's latest messages."))
                }
            } else {
                ProgressView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .safeAreaInset(edge: .bottom, spacing: 0) {
            if let history = store.workerHistory, history.daemon == daemon,
               history.sessionId == sessionId, history.childSessionId == childSessionId,
               !history.messages.isEmpty {
                RefreshStatusView(isLoading: history.isLoading, error: history.error, retry: refresh)
            }
        }
        .navigationTitle(name.isEmpty ? "Worker history" : name)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Refresh", systemImage: "arrow.clockwise", action: refresh)
                    .disabled(store.workerHistory?.isLoading == true)
            }
        }
        .task { refresh() }
        .onDisappear {
            if store.workerHistory?.childSessionId == childSessionId { store.workerHistory = nil }
        }
    }

    private func refresh() {
        store.openWorkerHistory(daemon: daemon, sessionId: sessionId, childSessionId: childSessionId)
    }
}

private struct JobOutputView: View {
    let daemon: String
    let sessionId: String
    let jobId: String
    let command: String
    @EnvironmentObject private var store: AppStore

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(command).font(.subheadline.monospaced()).textSelection(.enabled).padding(.horizontal)
            if let detail = store.jobOutput, detail.daemon == daemon,
               detail.sessionId == sessionId, detail.jobId == jobId {
                if let error = detail.error {
                    Label(error, systemImage: "exclamationmark.triangle")
                        .font(.callout).foregroundStyle(.secondary).padding(.horizontal)
                }
                if detail.incomplete {
                    Text("Some output could not be captured.").font(.caption).foregroundStyle(.secondary).padding(.horizontal)
                }
                ScrollView([.horizontal, .vertical]) {
                    if !detail.output.isEmpty {
                        Text(detail.output)
                            .font(.caption.monospaced()).textSelection(.enabled)
                            .fixedSize(horizontal: true, vertical: false).padding()
                    } else if !detail.isLoading {
                        Text(detail.done ? "No output was captured." : "No output yet.")
                            .font(.callout).foregroundStyle(.secondary).padding()
                    }
                }
                .defaultScrollAnchor(.topLeading)
                if detail.isLoading {
                    ProgressView("Loading output…").frame(maxWidth: .infinity)
                } else if detail.error != nil {
                    Button("Try again") { store.loadMoreJobOutput() }
                        .buttonStyle(.bordered)
                        .controlSize(.large)
                        .frame(maxWidth: .infinity)
                } else if detail.done {
                    Text("End of output")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                        .frame(maxWidth: .infinity)
                } else {
                    Button("Load more output") { store.loadMoreJobOutput() }
                        .buttonStyle(.bordered)
                        .controlSize(.large)
                        .frame(maxWidth: .infinity)
                }
            } else {
                ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity)
            }
        }
        .padding(.vertical)
        .navigationTitle("Job output")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .topBarTrailing) {
                Button("Reload output", systemImage: "arrow.clockwise") {
                    store.openJobOutput(daemon: daemon, sessionId: sessionId, jobId: jobId)
                }
                .accessibilityHint("Read output again from the beginning")
                .disabled(store.jobOutput?.isLoading == true)
            }
        }
        .task { store.openJobOutput(daemon: daemon, sessionId: sessionId, jobId: jobId) }
        .onDisappear {
            if store.jobOutput?.jobId == jobId { store.jobOutput = nil }
        }
    }
}
