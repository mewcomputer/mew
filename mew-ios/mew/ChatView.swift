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
        // Prefer the AI-generated (or first-prompt) session title from the
        // session list. Fall back to the session ID if the list hasn't
        // arrived yet.
        let daemonKey = store.selectedDaemonId?.nodeId ?? ""
        let sessionTitle = store.sessionLists[daemonKey]?
            .first(where: { $0.sessionId == sessionId })?
            .title
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let sessionTitle, !sessionTitle.isEmpty {
            return sessionTitle
        }
        return sessionId.count > 12 ? String(sessionId.prefix(8)) + "…" : sessionId
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

    // MARK: Composer (liquid glass chatbar)

    @ViewBuilder
    private var composer: some View {
        ChatBar(
            text: $inputText,
            focused: $composerFocused,
            model: currentModel,
            availableModels: store.availableModels,
            isStreaming: store.isStreaming,
            canSend: canSend,
            onSubmit: send,
            onPickModel: { model in
                store.switchModel(provider: model.provider, model: model.model)
                currentModel = model
            },
            onRefreshModels: { store.listModels() }
        )
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
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

    // MARK: Toolbar (intentionally empty — model picker lives in the chatbar)

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            EmptyView()
        }
    }

    @ViewBuilder
    private var modelPicker: some View {
        EmptyView()
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

// MARK: - ChatBar (liquid glass composer)

/// Bottom-anchored composer + model picker chip, rendered as a single
/// liquid glass capsule. The model picker used to live in the navigation
/// toolbar; moving it here keeps "interacting with the model" close to
/// the text field that produces the interaction.
struct ChatBar: View {
    @Binding var text: String
    var focused: FocusState<Bool>.Binding
    let model: ModelSummary?
    let availableModels: [ModelSummary]
    let isStreaming: Bool
    let canSend: Bool
    let onSubmit: () -> Void
    let onPickModel: (ModelSummary) -> Void
    let onRefreshModels: () -> Void

    @FocusState private var localFocus: Bool

    private let glassShape = RoundedRectangle(cornerRadius: 28, style: .continuous)

    var body: some View {
        VStack(spacing: 4) {
            // Row 1: textfield only
            textField
                .padding(.horizontal, 16)
                .padding(.top, 14)
                .padding(.bottom, 8)

            // Row 2: + | modelName | flex | submit
            HStack(spacing: 14) {
                attachmentsButton
                modelPickerChip
                Spacer(minLength: 12)
                sendOrCancelButton
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 12)
        }
        .background(
            glassShape
                .fill(.clear)
                .glassEffect(.regular.interactive(true), in: glassShape)
        )
        .onAppear { localFocus = focused.wrappedValue }
        .onChange(of: focused.wrappedValue) { _, new in
            localFocus = new
        }
    }

    @ViewBuilder
    private var textField: some View {
        TextField("Message mew…", text: $text, axis: .vertical)
            .lineLimit(1...6)
            .focused($localFocus)
            .submitLabel(.send)
            .onSubmit(onSubmit)
    }

    @ViewBuilder
    private var attachmentsButton: some View {
        // Placeholder for the attachments menu (image, file, project switch).
        // Disabled until attachments land; kept here so the layout is stable.
        Menu {
            Text("Attachments coming soon")
        } label: {
            Image(systemName: "plus")
                .font(.system(size: 16, weight: .semibold))
                .foregroundStyle(.secondary)
                .frame(width: 36, height: 36)
                .background(
                    Capsule().fill(.clear).glassEffect(.regular, in: Capsule())
                )
        }
    }

    @ViewBuilder
    private var modelPickerChip: some View {
        Menu {
            Section("Models") {
                ForEach(availableModels, id: \.id) { m in
                    Button {
                        onPickModel(m)
                    } label: {
                        HStack {
                            Text(modelLabel(m))
                            if model?.id == m.id {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
                if availableModels.isEmpty {
                    Text("No models loaded").foregroundStyle(.secondary)
                }
            }
            Section {
                Button {
                    onRefreshModels()
                } label: {
                    Label("Refresh models", systemImage: "arrow.clockwise")
                }
            }
        } label: {
            Text(model?.model ?? "Model")
                .font(.callout)
                .lineLimit(1)
                .padding(.horizontal, 14)
                .frame(height: 36)
                .background(
                    Capsule().fill(.clear).glassEffect(
                        .regular,
                        in: Capsule()
                    )
                )
        }
    }

    @ViewBuilder
    private var sendOrCancelButton: some View {
        if isStreaming {
            Button {
                onSubmit()
            } label: {
                Image(systemName: "stop.circle.fill")
                    .font(.system(size: 28))
                    .foregroundStyle(.red)
            }
            .accessibilityLabel("Stop generating")
        } else {
            Button {
                onSubmit()
            } label: {
                Image(systemName: "arrow.up.circle.fill")
                    .font(.system(size: 28))
                    .foregroundStyle(canSend ? .accentColor : Color(.tertiaryLabel))
            }
            .disabled(!canSend)
            .accessibilityLabel("Send")
        }
    }

    private func modelLabel(_ m: ModelSummary) -> String {
        if let window = m.contextWindow, window > 0 {
            return "\(m.provider)/\(m.model) · \(formatContext(window))"
        }
        return "\(m.provider)/\(m.model)"
    }

    private func formatContext(_ bytes: Int64) -> String {
        let kb = Double(bytes) / 1024
        if kb >= 1024 {
            return String(format: "%.0fK", kb / 1024) + " ctx"
        }
        return String(format: "%.0fK", kb) + " ctx"
    }
}
