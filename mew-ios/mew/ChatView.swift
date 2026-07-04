import SwiftUI
import MewMobileCore

// MARK: - ChatView

/// The main chat experience for a single daemon session.
///
/// Receives the daemon node id and session id, reads all conversation state from
/// the shared `AppStore`, and drives prompt sending, streaming display, model
/// selection, permission prompts, and ask-user sheets.
struct ChatView: View {
    let daemonNodeId: String
    let sessionId: String

    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss

    // Composer
    @State private var inputText: String = ""
    @FocusState private var composerFocused: Bool

    // Model picker (the store exposes no "current model", so we mirror the last
    // selection locally for the toolbar label).
    @State private var currentModel: ModelSummary?

    // Auto-scroll
    @State private var autoScroll: Bool = true
    @State private var containerHeight: CGFloat = 0

    // Loading state while waiting for session history replay
    @State private var isLoadingSession: Bool = false

    // Sheets
    @State private var permissionSheet: PermissionSheetItem?
    @State private var askUserSheet: AskUserSheetItem?

    var body: some View {
        VStack(spacing: 0) {
            if store.visibleMessages.isEmpty && !store.isStreaming && store.streamingText.isEmpty {
                if isLoadingSession {
                    ProgressView("Loading session…")
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    emptyState
                }
            } else {
                messageList
                composer
            }
        }
        .navigationTitle(title)
        .navigationBarTitleDisplayMode(.inline)
        .toolbar { toolbarContent }
        .task {
            store.selectSession(sessionId)
            isLoadingSession = true
            // Give the attach + history replay a moment to arrive
            try? await Task.sleep(for: .seconds(15))
            isLoadingSession = false
            store.listModels()
        }
        // Permission sheet
        .sheet(item: $permissionSheet) { item in
            PermissionSheet(permission: item.permission) { decision in
                store.respondPermission(requestId: item.permission.requestId, decision: decision)
            }
            .presentationDetents([.medium, .large])
        }
        // Ask-user sheet
        .sheet(item: $askUserSheet) { item in
            AskUserSheet(ask: item.ask) { answers in
                store.respondAskUser(requestId: item.ask.requestId, answers: answers)
            }
            .presentationDetents([.medium, .large])
        }
        // Drive sheets from the store's pending queues.
        .onChange(of: store.pendingPermissions.count) { _ in
            permissionSheet = store.pendingPermissions.last.map { PermissionSheetItem(permission: $0) }
        }
        .onChange(of: store.pendingAskUser.count) { _ in
            askUserSheet = store.pendingAskUser.last.map { AskUserSheetItem(ask: $0) }
        }
        // Mirror the first available model into the picker label.
        .onChange(of: store.availableModels) { models in
            if currentModel == nil { currentModel = models.first }
        }
        // Dismiss loading spinner as soon as messages arrive
        .onChange(of: store.visibleMessages) { _ in
            if !store.visibleMessages.isEmpty {
                isLoadingSession = false
            }
        }
    }

    private var title: String {
        sessionId.count > 12 ? String(sessionId.prefix(8)) + "…" : sessionId
    }

    // MARK: Message list

    @ViewBuilder
    private var messageList: some View {
        if store.messages.isEmpty && !store.isStreaming && store.streamingText.isEmpty {
            emptyState
        } else {
            scrollContent
        }
    }

    @ViewBuilder
    private var scrollContent: some View {
        ScrollView {
            ScrollViewReader { proxy in
                LazyVStack(alignment: .leading, spacing: 4) {
                    ForEach(store.visibleMessages, id: \.id) { message in
                        MessageItemView(message: message)
                    }

                    if showsStreamingBubble {
                        StreamingBubble(text: store.streamingText)
                            .id("streaming")
                    }

                    // Bottom sentinel used for auto-scroll + at-bottom detection.
                    Color.clear
                        .frame(height: 1)
                        .id("bottom")
                        .background(
                            GeometryReader { geo in
                                Color.clear.preference(
                                    key: BottomOffsetKey.self,
                                    value: geo.frame(in: .named("chatScroll")).minY
                                )
                            }
                        )
                }
                .padding(.vertical, 8)
                // Track whether the user is pinned to the bottom.
                .onPreferenceChange(BottomOffsetKey.self) { bottomY in
                    autoScroll = bottomY <= containerHeight + 80
                }
                // Auto-scroll on new messages and streaming deltas.
                .onChange(of: store.visibleMessages.count) { _ in
                    scrollToBottom(proxy: proxy)
                }
                .onChange(of: store.streamingText) { _ in
                    scrollToBottom(proxy: proxy)
                }
            }
        }
        .coordinateSpace(name: "chatScroll")
        .background(
            GeometryReader { geo in
                Color.clear
                    .onAppear { containerHeight = geo.size.height }
                    .onChange(of: geo.size.height) { containerHeight = $0 }
            }
        )
    }

    private var showsStreamingBubble: Bool {
        store.isStreaming || !store.streamingText.isEmpty
    }

    @ViewBuilder
    private var emptyState: some View {
        VStack(spacing: 14) {
            Image(systemName: "cat.fill")
                .font(.system(size: 48))
                .foregroundStyle(.tertiary)
            Text("mew")
                .font(.title2.bold())
            Text("Send a message to start the conversation.\nSlash commands like /help are passed to the daemon.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func scrollToBottom(proxy: ScrollViewProxy) {
        guard autoScroll else { return }
        withAnimation(.easeOut(duration: 0.2)) {
            proxy.scrollTo("bottom", anchor: .bottom)
        }
    }

    // MARK: Composer

    @ViewBuilder
    private var composer: some View {
        VStack(spacing: 0) {
            Divider()
            HStack(alignment: .bottom, spacing: 10) {
                TextField("Message mew…", text: $inputText, axis: .vertical)
                    .lineLimit(1...6)
                    .focused($composerFocused)
                    .submitLabel(.send)
                    .onSubmit { send() }

                sendOrCancelButton
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 10)
            .background(.bar)
        }
    }

    @ViewBuilder
    private var sendOrCancelButton: some View {
        if store.isStreaming {
            Button {
                store.cancelTurn()
            } label: {
                Image(systemName: "stop.circle.fill")
                    .font(.system(size: 28))
                    .foregroundStyle(.red)
            }
            .accessibilityLabel("Stop generating")
        } else {
            Button {
                send()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.system(size: 28))
                    .foregroundStyle(canSend ? .accentColor : Color(.tertiaryLabel))
            }
            .disabled(!canSend)
            .accessibilityLabel("Send")
        }
    }

    private var canSend: Bool {
        !inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && !store.isStreaming
    }

    private func send() {
        let trimmed = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !store.isStreaming else { return }
        // Slash commands and ordinary text are both passed straight through;
        // the daemon decides how to handle them.
        store.sendPrompt(trimmed)
        inputText = ""
    }

    // MARK: Toolbar (model picker)

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            modelPicker
        }
    }

    @ViewBuilder
    private var modelPicker: some View {
        Menu {
            Section("Models") {
                ForEach(store.availableModels, id: \.id) { model in
                    Button {
                        store.switchModel(provider: model.provider, model: model.model)
                        currentModel = model
                    } label: {
                        HStack {
                            Text(modelLabel(model))
                            if currentModel?.id == model.id {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
                if store.availableModels.isEmpty {
                    Text("No models loaded").foregroundStyle(.secondary)
                }
            }
            Section {
                Button {
                    store.listModels()
                } label: {
                    Label("Refresh models", systemImage: "arrow.clockwise")
                }
            }
        } label: {
            HStack(spacing: 4) {
                Image(systemName: "cpu")
                Text(currentModel?.model ?? "Model")
                    .lineLimit(1)
            }
            .font(.footnote)
        }
    }

    private func modelLabel(_ model: ModelSummary) -> String {
        if let window = model.contextWindow, window > 0 {
            return "\(model.provider)/\(model.model) · \(formatContext(window))"
        }
        return "\(model.provider)/\(model.model)"
    }

    private func formatContext(_ bytes: Int64) -> String {
        let kb = Double(bytes) / 1024
        if kb >= 1024 {
            return String(format: "%.0fK", kb / 1024) + " ctx"
        }
        return String(format: "%.0fK", kb) + " ctx"
    }
}

// MARK: - Preference key for bottom detection

private struct BottomOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat = .infinity
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

// MARK: - Permission sheet

private struct PermissionSheetItem: Identifiable {
    let permission: PendingPermission
    var id: UInt64 { permission.requestId }
}

private struct PermissionSheet: View {
    let permission: PendingPermission
    let onRespond: (Decision) -> Void

    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Label(permission.toolName, systemImage: "wrench.and.screwdriver.fill")
                        .font(.headline)

                    Text("wants to run a tool. Review the input below before approving.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)

                    VStack(alignment: .leading, spacing: 6) {
                        Text("Input")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.secondary)
                        Text(prettyJSON(permission.input))
                            .font(.caption.monospaced())
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 10, style: .continuous))
                    }
                }
                .padding(20)
            }
            .navigationTitle("Permission")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Deny", role: .destructive) { respond(.deny) }
                }
                ToolbarItemGroup(placement: .bottomBar) {
                    Button("Allow Session") { respond(.allowSession) }
                    Spacer()
                    Button("Allow Once") { respond(.allowOnce) }
                        .buttonStyle(.borderedProminent)
                }
            }
        }
    }

    private func respond(_ decision: Decision) {
        onRespond(decision)
        dismiss()
    }

    /// Pretty-prints a JSON input string, falling back to the raw text.
    private func prettyJSON(_ raw: String) -> String {
        guard let data = raw.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data, options: []),
              let pretty = try? JSONSerialization.data(
                withJSONObject: object,
                options: [.prettyPrinted, .sortedKeys]
              ),
              let result = String(data: pretty, encoding: .utf8) else {
            return raw
        }
        return result
    }
}

// MARK: - Ask-user sheet

private struct AskUserSheetItem: Identifiable {
    let ask: PendingAskUser
    var id: UInt64 { ask.requestId }
}

private struct AskUserSheet: View {
    let ask: PendingAskUser
    let onRespond: ([String]) -> Void

    @Environment(\.dismiss) private var dismiss
    @State private var answers: [String] = []

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text("The agent needs a few answers to continue.")
                        .font(.subheadline)
                        .foregroundStyle(.secondary)
                }
                ForEach(Array(ask.questions.enumerated()), id: \.offset) { index, question in
                    Section {
                        TextField(question, text: binding(at: index), axis: .vertical)
                            .lineLimit(1...4)
                    } header: {
                        Text(question)
                            .font(.caption.weight(.semibold))
                    }
                }
            }
            .navigationTitle("Questions")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Submit") { submit() }
                        .disabled(!allAnswered)
                }
            }
        }
        .onAppear {
            if answers.count != ask.questions.count {
                answers = Array(repeating: "", count: ask.questions.count)
            }
        }
    }

    private var allAnswered: Bool {
        !answers.contains { $0.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty }
    }

    private func binding(at index: Int) -> Binding<String> {
        Binding(
            get: { index < answers.count ? answers[index] : "" },
            set: { value in
                if index < answers.count {
                    answers[index] = value
                }
            }
        )
    }

    private func submit() {
        let trimmed = answers.map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
        onRespond(trimmed)
        dismiss()
    }
}
