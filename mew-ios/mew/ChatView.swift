import SwiftUI
import PhotosUI
import UniformTypeIdentifiers
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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme

    // Composer
    private var inputText: String {
        get { store.drafts[.chat(daemonNodeId: daemonNodeId, sessionId: sessionId)] ?? "" }
        nonmutating set {
            store.drafts[.chat(daemonNodeId: daemonNodeId, sessionId: sessionId)] = newValue.isEmpty ? nil : newValue
        }
    }
    @FocusState private var composerFocused: Bool

    private var currentModel: ModelSummary? { store.selectedModel(daemonNodeId: daemonNodeId) }

    private var ownsSelection: Bool {
        store.selectedDaemonId?.nodeId == daemonNodeId && store.selectedSessionId == sessionId
    }

    // Auto-scroll
    @State private var autoScroll: Bool = true
    @State private var containerHeight: CGFloat = 0
    @State private var historyAnchor: String?
    @State private var scrollToBottomTrigger = 0
    @State private var autoLoadHistory = HistoryAutoLoad()
    @State private var toolDisclosures = ToolDisclosures()

    // Sheets
    @State private var showsReview = false
    @State private var showsTools = false
    @State private var showsActivity = false
    @State private var showsDaemonSwitcher = false
    @State private var referencedFilePath: String?
    @State private var presentedArtifact: MessagePart?
    @State private var dismissedPromptIds = Set<String>()

    // File browser & attachments
    @State private var fileBrowserPath: String?
    @State private var showsPhotoPicker = false
    @State private var selectedPhotoItems: [PhotosPickerItem] = []
    @State private var showsFileImporter = false
    @State private var pendingAttachments: [StagedAttachment] = []
    @State private var attachmentError: String? = nil

    var body: some View {
        content
            .background(theme.background.ignoresSafeArea())
            // Float the composer over the content so messages scroll under the
            // liquid-glass chatbar; the inset doubles as the bottom scroll padding.
            .safeAreaInset(edge: .bottom) {
                VStack(spacing: 0) {
                    if let status = liveStatusText {
                        liveStatusBanner(status)
                    }
                    if showsRetryBanner {
                        retryBanner
                    }
                    if ownsSelection, let queued = store.queuedMessages[sessionId], !queued.isEmpty {
                        queuedMessagesBanner(queued)
                    }
                    composer
                }
                .background(alignment: .bottom) {
                    LinearGradient(
                        colors: [
                            theme.background.opacity(0),
                            theme.background.opacity(0.88),
                            theme.background
                        ],
                        startPoint: .top,
                        endPoint: .bottom
                    )
                    .padding(.top, -24)
                    .ignoresSafeArea(edges: .bottom)
                    .allowsHitTesting(false)
                }
            }
            // Todo panel above the content when todos exist for this session.
            .safeAreaInset(edge: .top) {
                VStack(spacing: 0) {
                    if store.daemonStatuses[daemonNodeId] != .connected {
                        HStack {
                            DaemonConnectionStatusView(status: store.daemonStatuses[daemonNodeId] ?? .disconnected)
                            Spacer(minLength: 8)
                            NavigationLink("Details", value: NavigationRoute.daemonDetails(daemonNodeId: daemonNodeId))
                                .frame(minHeight: 44)
                        }
                        .font(.callout)
                        .padding(.horizontal, 16)
                        .background(Color(.secondarySystemBackground))
                    }
                    if ownsSelection, let todos = store.todos[sessionId], !todos.isEmpty {
                        TodoPanelView(todos: todos)
                    }
                }
            }
            .animation(
                Theme.Motion.value(Theme.Motion.surface, reduced: reduceMotion),
                value: showsRetryBanner
            )
            .navigationTitle("")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar { toolbarContent }
            .onAppear {
                store.listModels()
                store.listPersonas()
                store.listMentionCandidates()
            }
            .sheet(item: Binding(
                get: { activePromptSheet },
                set: { newValue in
                    if newValue == nil, let current = activePromptSheet {
                        dismissedPromptIds.insert(current.id)
                    }
                }
            )) { prompt in
                switch prompt {
                case .permission(let permission):
                    PermissionSheet(permission: permission) { decision in
                        store.respondPermission(requestId: permission.requestId, decision: decision)
                    }
                    .presentationDetents([.medium, .large])
                    .presentationBackgroundInteraction(.enabled(upThrough: .medium))
                case .askUser(let ask):
                    AskUserSheet(ask: ask, onRespond: { answers in
                        store.respondAskUser(requestId: ask.requestId, answers: answers)
                    }, onCancel: {
                        store.cancelAskUser(requestId: ask.requestId)
                    })
                    .presentationDetents([.medium, .large])
                    .presentationBackgroundInteraction(.enabled(upThrough: .medium))
                case .goal(let goal):
                    GoalProposalSheet(objective: goal.objective) { accepted in
                        store.respondToGoal(requestId: goal.requestId, accepted: accepted)
                    }
                    .presentationDetents([.medium, .large])
                    .presentationBackgroundInteraction(.enabled(upThrough: .medium))
                }
            }
            .sheet(item: Binding(
                get: { fileBrowserPath.map { FileBrowserItem(path: $0) } },
                set: { newValue in fileBrowserPath = newValue?.path }
            )) { item in
                FileBrowserSheet(
                    sessionId: sessionId,
                    currentPath: item.path,
                    onNavigate: { newPath in fileBrowserPath = newPath },
                    onPick: { path in
                        let mentionPath: String
                        if let cwd = initialBrowserPath(), !cwd.isEmpty {
                            let prefix = cwd.hasSuffix("/") ? cwd : cwd + "/"
                            mentionPath = path.hasPrefix(prefix) ? String(path.dropFirst(prefix.count)) : path
                        } else {
                            mentionPath = path
                        }
                        if !inputText.isEmpty, !inputText.hasSuffix(" ") {
                            inputText += " "
                        }
                        inputText += "@\(mentionPath) "
                        fileBrowserPath = nil
                    },
                    onClose: { fileBrowserPath = nil }
                )
                .presentationDetents([.large])
            }
            .sheet(item: Binding(
                get: { referencedFilePath.map { FileBrowserItem(path: $0) } },
                set: { referencedFilePath = $0?.path }
            )) { item in
                FilePreviewSheet(sessionId: sessionId, path: item.path)
                    .presentationDetents([.large])
            }
            .sheet(item: Binding(
                get: { presentedArtifact.map { ArtifactSheetItem(part: $0) } },
                set: { presentedArtifact = $0?.part }
            )) { item in
                ArtifactSheetView(part: item.part)
                    .presentationDetents([.large])
                    .presentationDragIndicator(.visible)
            }
            .sheet(isPresented: $showsReview) {
                ReviewView(daemon: daemonNodeId, sessionId: sessionId)
            }
            .sheet(isPresented: $showsTools) {
                ToolHistoryView(messages: store.messages)
            }
            .sheet(isPresented: $showsActivity) {
                ActivityView(daemon: daemonNodeId, sessionId: sessionId)
            }
            .sheet(isPresented: $showsDaemonSwitcher) {
                DaemonSwitcherView { daemon in
                    showsDaemonSwitcher = false
                    store.openDaemon(daemon)
                }
            }
            .photosPicker(
                isPresented: $showsPhotoPicker,
                selection: $selectedPhotoItems,
                maxSelectionCount: 5,
                matching: .any(of: [.images])
            )
            .onChange(of: selectedPhotoItems) { _, items in
                guard !items.isEmpty else { return }
                Task { await loadPickedPhotos(items) }
            }
            .fileImporter(
                isPresented: $showsFileImporter,
                allowedContentTypes: [.item],
                allowsMultipleSelection: true
            ) { result in
                handleFileImport(result)
            }
    }

    @ViewBuilder
    private var content: some View {
        if !ownsSelection {
            ProgressView("Loading conversation…")
                .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if let error = store.sessionLoadError {
            ContentUnavailableView {
                Label("Couldn't open this chat", systemImage: "exclamationmark.bubble")
            } description: {
                Text(error)
            } actions: {
                Button("Try again") {
                    store.retrySessionAttachment(daemonNodeId: daemonNodeId, sessionId: sessionId)
                }
                .buttonStyle(.borderedProminent)
                .disabled(store.daemonStatuses[daemonNodeId] != .connected)
                NavigationLink("Connection details", value: NavigationRoute.daemonDetails(daemonNodeId: daemonNodeId))
                    .frame(minHeight: 44)
            }
        } else if store.conversationItems.isEmpty && store.historyBefore == nil && !store.isStreaming && store.streamingText.isEmpty && !store.isCompacting {
            if store.isLoadingSession {
                ProgressView("Loading session…")
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            } else {
                emptyState
            }
        } else {
            messageList
        }
    }

    private var title: String {
        // Prefer the AI-generated (or first-prompt) session title from the
        // session list. Fall back to the session ID if the list hasn't
        // arrived yet.
        let sessionTitle = store.sessionLists[daemonNodeId]?
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
        if store.messages.isEmpty && store.historyBefore == nil && !store.isStreaming && store.streamingText.isEmpty {
            emptyState
        } else {
            scrollContent
        }
    }

    @ViewBuilder
    private var scrollContent: some View {
        let items = store.conversationItems
        ScrollView {
            ScrollViewReader { proxy in
                // Non-lazy so each message's MarkdownView parses once on load and
                // isn't re-parsed (with a layout jump) every time it scrolls back
                // into view. Trades more upfront work in long sessions for
                // smoother scrolling.
                VStack(alignment: .leading, spacing: 4) {
                    if store.historyBefore != nil {
                        VStack(spacing: 6) {
                            Button {
                                loadOlderHistory(anchoredAt: items.first?.sourceMessageIds.first)
                            } label: {
                                if store.isLoadingOlderHistory {
                                    ProgressView("Loading older messages…")
                                } else {
                                    Label("Load older messages", systemImage: "arrow.up")
                                }
                            }
                            .disabled(store.isLoadingOlderHistory)
                            if let error = store.historyError {
                                Text(error)
                                    .font(.caption)
                                    .foregroundStyle(.secondary)
                            }
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 12)
                    }
                    ForEach(items) { item in
                        MessageItemView(
                            message: item.message,
                            fontChoice: store.fontChoice,
                            onOpenFile: { referencedFilePath = $0 },
                            onOpenArtifact: { presentedArtifact = $0 },
                            disclosures: $toolDisclosures
                        )
                    }

                    if showsStreamingBubble {
                        StreamingBubble(
                            text: store.streamingText,
                            fontChoice: store.fontChoice,
                            onOpenFile: { referencedFilePath = $0 }
                        )
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
                // Backfill as the reader drags up past the oldest loaded
                // message; the button above stays for a deliberate retry.
                .onScrollGeometryChange(for: CGFloat.self) { geometry in
                    geometry.contentOffset.y + geometry.contentInsets.top
                } action: { previous, current in
                    guard autoLoadHistory.scrollsIntoBackfill(from: previous, to: current) else { return }
                    loadOlderHistory(anchoredAt: items.first?.sourceMessageIds.first)
                }
                // Auto-scroll on any content change (new messages, streaming
                // deltas, and growing reasoning/tool output within a message),
                // not just when the message count changes.
                .onChange(of: store.conversationItems) {
                    if let anchor = historyAnchor {
                        if !store.isLoadingOlderHistory {
                            let target = items.first { $0.sourceMessageIds.contains(anchor) }?.id ?? anchor
                            proxy.scrollTo(target, anchor: .top)
                            historyAnchor = nil
                        }
                    } else {
                        scrollToBottom(proxy: proxy)
                    }
                }
                .onChange(of: store.historyError) {
                    if store.historyError != nil { historyAnchor = nil }
                }
                // Streaming deltas arrive rapidly; scroll without animation so
                // the growing text and an animated scroll don't fight (jitter).
                .onChange(of: store.streamingText) {
                    scrollToBottom(proxy: proxy, animated: false)
                }
                .onChange(of: scrollToBottomTrigger) {
                    scrollToBottom(proxy: proxy)
                }
            }
        }
        .scrollDismissesKeyboard(.interactively)
        .coordinateSpace(name: "chatScroll")
        .background(
            GeometryReader { geo in
                Color.clear
                    .onAppear { containerHeight = geo.size.height }
                    .onChange(of: geo.size.height) { containerHeight = geo.size.height }
            }
        )
        .overlay(alignment: .bottom) {
            if !autoScroll && !items.isEmpty {
                Button {
                    autoScroll = true
                    scrollToBottomTrigger += 1
                } label: {
                    Image(systemName: "arrow.down")
                        .font(.system(size: 14, weight: .semibold))
                        .foregroundStyle(.primary)
                        .frame(width: 34, height: 34)
                        .background(
                            Circle()
                                .fill(.clear)
                                .glassEffect(.regular.interactive(true), in: Circle())
                        )
                }
                .buttonStyle(MewPressButtonStyle())
                .accessibilityLabel("Jump to bottom")
                .padding(.bottom, 10)
                .transition(reduceMotion ? .opacity : .scale(scale: 0.85).combined(with: .opacity))
            }
        }
        .animation(Theme.Motion.value(Theme.Motion.surface, reduced: reduceMotion), value: autoScroll)
    }

    private var showsStreamingBubble: Bool {
        !store.streamingText.isEmpty
    }

    private var liveStatusText: String? {
        guard ownsSelection else { return nil }
        if store.isCompacting {
            return "Compacting context…"
        }
        if let retry = store.retryWait {
            return "Retrying (\(retry.attempt)/\(retry.maxAttempts)) in \(retry.delaySecs)s — \(retry.reason)"
        }
        if store.pendingPermissions.contains(where: { $0.daemon == daemonNodeId && $0.sessionId == sessionId }) {
            return "Waiting for approval"
        }
        if store.pendingAskUser.contains(where: { $0.daemon == daemonNodeId && $0.sessionId == sessionId }) {
            return "Waiting for answer"
        }
        if store.pendingGoals.contains(where: { $0.daemon == daemonNodeId && $0.sessionId == sessionId }) {
            return "Goal proposed"
        }
        let runningWorkers = store.subagents.filter { $0.status == "running" }
        if runningWorkers.count == 1, let single = runningWorkers.first {
            if let tool = single.toolName, !tool.isEmpty {
                let name = single.displayName ?? single.name
                return "\(name): \(tool)"
            }
            return "Waiting on 1 subagent"
        }
        if !runningWorkers.isEmpty {
            return "Waiting on \(runningWorkers.count) subagents"
        }
        if store.isStreaming {
            return "Working…"
        }
        return nil
    }

    private var liveMascotState: MascotState {
        if store.retryWait != nil { return .retrying }
        if rawPendingPromptSheet != nil {
            return .waiting
        }
        return .working
    }

    @ViewBuilder
    private func liveStatusBanner(_ status: String) -> some View {
        let hasPendingPrompt = rawPendingPromptSheet != nil
        Button {
            if hasPendingPrompt {
                dismissedPromptIds.removeAll()
            }
        } label: {
            HStack(spacing: 8) {
                MascotView(state: liveMascotState, size: 22)
                Text(status)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                Spacer(minLength: 0)
                if hasPendingPrompt {
                    Text("Tap to open")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(Color.accentColor)
                }
            }
            .padding(.horizontal, 14)
            .padding(.vertical, 6)
            .background(
                RoundedRectangle(cornerRadius: 12, style: .continuous)
                    .fill(.ultraThinMaterial)
            )
        }
        .buttonStyle(.plain)
        .disabled(!hasPendingPrompt)
        .padding(.horizontal, 12)
        .padding(.bottom, 6)
    }

    /// Whether the retry banner should appear: only when the latest turn for
    /// this session failed, the agent isn't currently streaming, and there's
    /// message content to show below.
    private var showsRetryBanner: Bool {
        guard !store.isStreaming else { return false }
        return store.lastTurnFailed[sessionId] == true
    }

    /// Retry is enabled when we have a stored prompt to re-send. When the
    /// session was attached fresh after a failure (no Swift-side prompt
    /// captured), the button is disabled.
    private var canRetry: Bool {
        store.canSendPrompt && (store.lastPrompt[sessionId]?.isEmpty == false)
    }

    @ViewBuilder
    private var retryBanner: some View {
        HStack(spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
            VStack(alignment: .leading, spacing: 1) {
                Text("Turn failed")
                    .font(.subheadline.weight(.semibold))
                Text(canRetry ? "Tap Retry to send the last message again."
                              : "The last turn failed. No prompt to retry.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer(minLength: 8)
            Button("Retry") {
                store.retryLastTurn()
            }
            .buttonStyle(.borderedProminent)
            .controlSize(.small)
            .disabled(!canRetry)
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 10)
        .background(
            theme.card
        )
        .transition(reduceMotion ? .opacity : .move(edge: .bottom).combined(with: .opacity))
    }

    @ViewBuilder
    private var emptyState: some View {
        let machine = store.daemons.first(where: { $0.nodeId == daemonNodeId })?.name
        let folder = store.sessionLists[daemonNodeId]?
            .first(where: { $0.sessionId == sessionId })?
            .cwd
            .map { ($0 as NSString).lastPathComponent }
        let contextLabel = [machine, folder]
            .compactMap { $0 }
            .filter { !$0.isEmpty }
            .joined(separator: " · ")

        VStack(spacing: 12) {
            MascotView(state: .working, size: 56)
            Text("What can we work on?")
                .font(.title2.weight(.semibold))
                .tracking(-0.3)
            if !contextLabel.isEmpty {
                Text(contextLabel)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
            Text("Send a message or type / for commands.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private func scrollToBottom(proxy: ScrollViewProxy, animated: Bool = true) {
        guard autoScroll else { return }
        if animated {
            withAnimation(Theme.Motion.value(.easeOut(duration: 0.2), reduced: reduceMotion)) {
                proxy.scrollTo("bottom", anchor: .bottom)
            }
        } else {
            proxy.scrollTo("bottom", anchor: .bottom)
        }
    }

    /// Request one older page, keeping the reader's position on `anchor` so the
    /// transcript does not jump when the page arrives.
    private func loadOlderHistory(anchoredAt anchor: String?) {
        historyAnchor = anchor
        autoScroll = false
        if !store.loadOlderHistory() {
            historyAnchor = nil
            autoLoadHistory.rearm()
        }
    }

    @ViewBuilder
    private func queuedMessagesBanner(_ queued: [MobileQueuedMessage]) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Queued (\(queued.count))")
                .font(.caption2.weight(.semibold))
                .foregroundStyle(.secondary)
                .padding(.horizontal, 4)

            ForEach(queued, id: \.id) { item in
                HStack(spacing: 8) {
                    Text(item.steer ? "Steer" : "Queued")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(item.steer ? Color.orange : Color.secondary)
                        .padding(.horizontal, 6)
                        .padding(.vertical, 2)
                        .background(
                            Capsule().fill(
                                item.steer ? Color.orange.opacity(0.16) : Color(.tertiarySystemFill)
                            )
                        )

                    Text(item.text)
                        .font(.caption)
                        .lineLimit(2)
                        .frame(maxWidth: .infinity, alignment: .leading)

                    if !item.steer {
                        Button("Steer") {
                            store.promoteQueuedMessage(id: item.id)
                        }
                        .font(.caption2.weight(.semibold))
                        .buttonStyle(.bordered)
                        .controlSize(.mini)
                    }

                    Button {
                        store.removeQueuedMessage(id: item.id)
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 16))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Remove queued message")
                }
                .padding(.horizontal, 10)
                .padding(.vertical, 6)
                .background(
                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                        .fill(Color(.secondarySystemBackground))
                )
            }
        }
        .padding(8)
        .background(
            RoundedRectangle(cornerRadius: 16, style: .continuous)
                .fill(.clear)
                .glassEffect(.regular, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        )
        .padding(.horizontal, 12)
        .padding(.bottom, 6)
    }

    // MARK: Composer (liquid glass chatbar)

    @ViewBuilder
    private var composer: some View {
        let daemonId = daemonNodeId
        ChatBar(
            text: Binding(get: { inputText }, set: { inputText = $0 }),
            focused: $composerFocused,
            model: currentModel,
            availableModels: store.modelsByDaemon[daemonNodeId] ?? [],
            isStreaming: store.isStreaming,
            isBusy: store.isStreaming || store.hasPendingSubmission,
            canSend: canSend,
            canEnqueue: canEnqueue,
            onSubmit: send,
            onSteer: steer,
            onCancel: { store.cancelTurn() },
            onPickModel: { model in
                store.switchModel(provider: model.provider, model: model.model)
            },
            onRefreshModels: { store.listModels() },
            onAttachments: { fileBrowserPath = initialBrowserPath() },
            permissionMode: store.permissionMode[daemonId],
            onPickPermissionMode: { mode in store.setPermissionMode(mode) },
            thinkingVariant: store.thinkingVariant[daemonId] ?? nil,
            onPickThinkingVariant: { variant in store.setThinkingVariant(variant) },
            // Persona
            currentPersona: store.currentPersona[daemonId],
            availablePersonas: store.availablePersonas[daemonId] ?? [],
            onPickPersona: { name in store.switchPersona(name: name) },
            usage: store.sessionUsage[sessionId],
            directoryListings: store.directoryListings,
            sessionId: sessionId,
            sessionCwd: initialBrowserPath(),
            onRequestDirectory: { path in
                store.fetchDirectory(sessionId: sessionId, path: path)
            },
            availableSkills: store.availableSkills[daemonId] ?? [],
            availableSubagents: store.availableSubagents[daemonId] ?? [],
            pendingAttachments: pendingAttachments,
            attachmentError: attachmentError,
            onRemoveAttachment: { id in pendingAttachments.removeAll { $0.id == id } },
            onPickPhotos: { showsPhotoPicker = true },
            onPickFiles: { showsFileImporter = true },
            onDismissAttachmentError: { attachmentError = nil }
        )
        .disabled(!ownsSelection)
        .padding(.horizontal, 12)
        .padding(.bottom, 8)
    }

    /// Default the file browser to the session's cwd (if set) or
    /// fall back to the home directory. The daemon resolves and
    /// returns the listing.
    private func initialBrowserPath() -> String? {
        if let cwd = store.sessionLists[store.selectedDaemonId?.nodeId ?? ""]?
            .first(where: { $0.sessionId == sessionId })?
            .cwd
        {
            return cwd
        }
        return nil // daemon defaults to its cwd
    }

    private var rawPendingPromptSheet: ActivePromptSheet? {
        if let perm = store.pendingPermissions.first(where: { $0.daemon == daemonNodeId && $0.sessionId == sessionId }) {
            return .permission(perm)
        }
        if let ask = store.pendingAskUser.first(where: { $0.daemon == daemonNodeId && $0.sessionId == sessionId }) {
            return .askUser(ask)
        }
        if let goal = store.pendingGoals.first(where: { $0.daemon == daemonNodeId && $0.sessionId == sessionId }) {
            return .goal(goal)
        }
        return nil
    }

    private var activePromptSheet: ActivePromptSheet? {
        guard let prompt = rawPendingPromptSheet, !dismissedPromptIds.contains(prompt.id) else {
            return nil
        }
        return prompt
    }

    private var canSend: Bool {
        let hasText = !inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
        return ownsSelection && (hasText || !pendingAttachments.isEmpty) && store.canSendPrompt
    }

    private var canEnqueue: Bool {
        ownsSelection && !inputText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && store.canEnqueuePrompt
    }

    private func send() {
        let trimmed = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty || !pendingAttachments.isEmpty else { return }
        if store.isStreaming || store.hasPendingSubmission {
            guard !trimmed.isEmpty, store.enqueueMessage(trimmed, steer: false) else { return }
            inputText = ""
            return
        }
        guard store.canSendPrompt else { return }
        if trimmed.hasPrefix("/") && pendingAttachments.isEmpty {
            // Slash commands go to the daemon's slash handler, not the LLM.
            guard store.sendSlashCommand(trimmed) else { return }
        } else {
            let wireAttachments = pendingAttachments.map {
                PromptAttachment(path: $0.path, mime: $0.mime)
            }
            guard store.sendPrompt(trimmed, attachments: wireAttachments) else { return }
        }
        inputText = ""
        pendingAttachments = []
        attachmentError = nil
    }

    private func loadPickedPhotos(_ items: [PhotosPickerItem]) async {
        for (idx, item) in items.enumerated() {
            let contentType = item.supportedContentTypes.first
            let mime = contentType?.preferredMIMEType ?? "image/jpeg"
            let ext = contentType?.preferredFilenameExtension ?? "jpg"
            guard let data = try? await item.loadTransferable(type: Data.self) else {
                await MainActor.run {
                    attachmentError = "Couldn't read selected image."
                }
                continue
            }
            await MainActor.run {
                stagePickedData(data, filename: "photo-\(idx + 1).\(ext)", mime: mime)
            }
        }
        await MainActor.run {
            selectedPhotoItems = []
        }
    }

    private func handleFileImport(_ result: Result<[URL], Error>) {
        switch result {
        case .failure(let error):
            attachmentError = error.localizedDescription
        case .success(let urls):
            for url in urls {
                let accessed = url.startAccessingSecurityScopedResource()
                defer { if accessed { url.stopAccessingSecurityScopedResource() } }
                do {
                    let data = try Data(contentsOf: url)
                    let mime = UTType(filenameExtension: url.pathExtension)?.preferredMIMEType ?? "application/octet-stream"
                    stagePickedData(data, filename: url.lastPathComponent, mime: mime)
                } catch {
                    attachmentError = "Couldn't read \(url.lastPathComponent): \(error.localizedDescription)"
                }
            }
        }
    }

    private func stagePickedData(_ data: Data, filename: String, mime: String) {
        let maxBytes = 10 * 1024 * 1024
        guard data.count <= maxBytes else {
            let formatted = ByteCountFormatter.string(fromByteCount: Int64(maxBytes), countStyle: .file)
            attachmentError = "\(filename) exceeds the \(formatted) upload limit."
            return
        }
        let dataUrl = "data:\(mime);base64,\(data.base64EncodedString())"
        pendingAttachments.append(
            StagedAttachment(name: filename, path: dataUrl, mime: mime, byteCount: data.count)
        )
        attachmentError = nil
    }

    private func steer() {
        let trimmed = inputText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, store.enqueueMessage(trimmed, steer: true) else { return }
        inputText = ""
    }

    // MARK: Toolbar

    @ToolbarContentBuilder
    private var toolbarContent: some ToolbarContent {
        ToolbarItem(placement: .topBarTrailing) {
            Button("Review changes", systemImage: "doc.text.magnifyingglass") {
                showsReview = true
            }
        }
        ToolbarItem(placement: .topBarTrailing) {
            Menu {
                Button("Tools", systemImage: "wrench.and.screwdriver") {
                    showsTools = true
                }
                Button("Workers & jobs", systemImage: "person.2") {
                    showsActivity = true
                }
                Button("Switch machine", systemImage: "arrow.left.arrow.right") {
                    showsDaemonSwitcher = true
                }
                NavigationLink(value: NavigationRoute.daemonDetails(daemonNodeId: daemonNodeId)) {
                    Label("Machine details", systemImage: "server.rack")
                }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
        }
    }
}

// MARK: - Preference key for bottom detection

private struct BottomOffsetKey: PreferenceKey {
    static var defaultValue: CGFloat = .infinity
    static func reduce(value: inout CGFloat, nextValue: () -> CGFloat) {
        value = nextValue()
    }
}

/// Arms backfill when the reader drags up out of loaded history. One page per
/// upward gesture keeps a flick from paging a whole session, and the store
/// drops overlapping requests, so this only has to decide intent.
struct HistoryAutoLoad: Equatable {
    /// Distance from the top of the content at which a pull counts as
    /// "out of loaded history".
    static let triggerDistance: CGFloat = 320

    private(set) var armed = true

    /// Whether a scroll from `previous` to `current` should request the next
    /// page. Both distances are measured from the top of the content, so 0
    /// means the reader is at the oldest loaded message.
    mutating func scrollsIntoBackfill(from previous: CGFloat, to current: CGFloat) -> Bool {
        // Coming back down re-arms nothing on its own; leaving the trigger
        // zone does, so a reader who stopped at the top is not paged again
        // until they have moved away and pulled up a second time.
        if current > Self.triggerDistance {
            armed = true
            return false
        }
        guard armed, current < previous - 1 else { return false }
        armed = false
        return true
    }

    /// Re-arm after a refused request so a transient disconnect does not leave
    /// backfill dead until the reader scrolls away and back.
    mutating func rearm() {
        armed = true
    }
}

// MARK: - Prompt sheets

private enum ActivePromptSheet: Identifiable {
    case permission(PendingPermission)
    case askUser(PendingAskUser)
    case goal(PendingGoalProposal)

    var id: String {
        switch self {
        case .permission(let p): return "perm:\(p.requestId)"
        case .askUser(let a):    return "ask:\(a.requestId)"
        case .goal(let g):       return "goal:\(g.requestId)"
        }
    }
}

private struct PermissionSheet: View {
    let permission: PendingPermission
    let onRespond: (Decision) -> Void

    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme

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
                            .font(.mewMono(12))
                            .textSelection(.enabled)
                            .frame(maxWidth: .infinity, alignment: .leading)
                            .padding(10)
                            .background(theme.toolBackground, in: RoundedRectangle(cornerRadius: Theme.Layout.controlRadius, style: .continuous))
                    }
                }
                .padding(20)
            }
            .background(theme.background.ignoresSafeArea())
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

private struct GoalProposalSheet: View {
    let objective: String
    let onRespond: (Bool) -> Void

    @Environment(\.mewTheme) private var theme

    var body: some View {
        NavigationStack {
            ScrollView {
                Text(objective)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(20)
            }
            .background(theme.background.ignoresSafeArea())
            .navigationTitle("Proposed goal")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Reject", role: .destructive) { onRespond(false) }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Accept") { onRespond(true) }
                }
            }
        }
    }
}

// MARK: - Ask-user sheet

private struct AskUserSheet: View {
    let ask: PendingAskUser
    let onRespond: ([String]) -> Void
    let onCancel: () -> Void

    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme
    @State private var answers: [String] = []

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    if let subagent = ask.subagent {
                        HStack(spacing: 8) {
                            SubagentIdenticonView(
                                seed: subagent.childSessionId.isEmpty ? (subagent.displayName ?? subagent.name) : subagent.childSessionId,
                                size: 18
                            )
                            Text("\(subagent.displayName ?? subagent.name) (\(subagent.name)) needs your input.")
                                .font(.subheadline)
                                .foregroundStyle(.secondary)
                        }
                    } else {
                        Text("The agent needs a few answers to continue.")
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
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
                .listRowBackground(theme.card)
            }
            .mewThemedForm(theme)
            .navigationTitle("Questions")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button(ask.subagent == nil ? "Cancel" : "Close question") {
                        if ask.subagent != nil { onCancel() }
                        dismiss()
                    }
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
    let isBusy: Bool
    let canSend: Bool
    let canEnqueue: Bool
    let onSubmit: () -> Void
    let onSteer: () -> Void
    let onCancel: () -> Void
    let onPickModel: (ModelSummary) -> Void
    let onRefreshModels: () -> Void
    let onAttachments: () -> Void
    // Permission mode
    let permissionMode: String?
    let onPickPermissionMode: (String) -> Void
    // Thinking variant
    let thinkingVariant: String?
    let onPickThinkingVariant: (String) -> Void
    // Persona
    let currentPersona: String?
    let availablePersonas: [PersonaInfo]
    let onPickPersona: (String) -> Void
    // Usage
    let usage: SessionUsage?
    // Mention autocomplete
    var directoryListings: [String: DirListing] = [:]
    var sessionId: String = ""
    var sessionCwd: String? = nil
    var onRequestDirectory: (String?) -> Void = { _ in }
    var availableSkills: [MentionCandidate] = []
    var availableSubagents: [MentionCandidate] = []
    // Staged file/image attachments
    var pendingAttachments: [StagedAttachment] = []
    var attachmentError: String? = nil
    var onRemoveAttachment: (UUID) -> Void = { _ in }
    var onPickPhotos: () -> Void = {}
    var onPickFiles: () -> Void = {}
    var onDismissAttachmentError: () -> Void = {}

    @FocusState private var localFocus: Bool
    @State private var showsModelPicker = false
    @Environment(\.accessibilityReduceTransparency) private var reduceTransparency
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme

    private let glassShape = RoundedRectangle(cornerRadius: 28, style: .continuous)

    private var isExpanded: Bool {
        localFocus || !text.isEmpty || isBusy || !pendingAttachments.isEmpty
    }

    private struct MentionSuggestion: Identifiable {
        let id: String
        let insertText: String
        let label: String
        let detail: String?
        let icon: String
        let appendSpace: Bool
    }

    private var activeMentionQuery: String? {
        guard let lastChar = text.last, !lastChar.isWhitespace else { return nil }
        guard let token = text.split(omittingEmptySubsequences: false, whereSeparator: \.isWhitespace).last,
              token.hasPrefix("@") else {
            return nil
        }
        return String(token.dropFirst())
    }

    private var mentionPathComponents: (dirPrefix: String, leafQuery: String, resolvedDirPath: String?)? {
        guard let query = activeMentionQuery else { return nil }
        if query.hasPrefix("model:") || query.hasPrefix("skill:") || query.hasPrefix("subagent:") {
            return ("", query, sessionCwd)
        }
        if let slashIdx = query.lastIndex(of: "/") {
            let prefix = String(query[..<slashIdx])
            let leaf = String(query[query.index(after: slashIdx)...])
            let resolved: String?
            if let cwd = sessionCwd, !cwd.isEmpty {
                resolved = prefix.isEmpty ? cwd : (cwd as NSString).appendingPathComponent(prefix)
            } else {
                resolved = prefix.isEmpty ? nil : prefix
            }
            return (prefix, leaf, resolved)
        }
        return ("", query, sessionCwd)
    }

    private var mentionDirectoryKey: String? {
        guard let info = mentionPathComponents else { return nil }
        return "\(sessionId)::\(info.resolvedDirPath ?? "")"
    }

    private var mentionSuggestions: [MentionSuggestion] {
        guard let info = mentionPathComponents else { return [] }
        let entries = mentionDirectoryKey.flatMap { directoryListings[$0]?.entries } ?? []
        var candidates: [MentionSuggestion] = []
        for entry in entries {
            let rel = info.dirPrefix.isEmpty ? entry.name : "\(info.dirPrefix)/\(entry.name)"
            let value = entry.isDir ? "\(rel)/" : rel
            candidates.append(MentionSuggestion(
                id: "file:\(value)",
                insertText: value,
                label: "@\(value)",
                detail: nil,
                icon: entry.isDir ? "folder" : "doc.text",
                appendSpace: !entry.isDir
            ))
        }
        if info.dirPrefix.isEmpty {
            for s in availableSkills {
                let value = "skill:\(s.name)"
                candidates.append(MentionSuggestion(
                    id: "skill:\(s.name)",
                    insertText: value,
                    label: "@\(value)",
                    detail: s.description.isEmpty ? nil : s.description,
                    icon: "sparkles",
                    appendSpace: true
                ))
            }
            for s in availableSubagents {
                let value = "subagent:\(s.name)"
                candidates.append(MentionSuggestion(
                    id: "subagent:\(s.name)",
                    insertText: value,
                    label: "@\(value)",
                    detail: s.description.isEmpty ? nil : s.description,
                    icon: "person.2",
                    appendSpace: true
                ))
            }
            for m in availableModels {
                let value = "model:\(m.provider)/\(m.model)"
                candidates.append(MentionSuggestion(
                    id: "model:\(m.id)",
                    insertText: value,
                    label: "@\(value)",
                    detail: m.displayName,
                    icon: "cpu",
                    appendSpace: true
                ))
            }
        }
        let leaf = info.leafQuery
        if leaf.isEmpty {
            return Array(candidates.prefix(5))
        }
        return Array(candidates.filter {
            $0.insertText.localizedCaseInsensitiveContains(leaf)
                || $0.label.localizedCaseInsensitiveContains(leaf)
                || ($0.detail?.localizedCaseInsensitiveContains(leaf) == true)
        }.prefix(5))
    }

    private func applyMention(_ suggestion: MentionSuggestion) {
        guard let atIndex = text.lastIndex(of: "@") else { return }
        let suffix = suggestion.appendSpace ? " " : ""
        text.replaceSubrange(atIndex..., with: "@\(suggestion.insertText)\(suffix)")
    }

    var body: some View {
        VStack(spacing: 6) {
            if !mentionSuggestions.isEmpty {
                mentionPopover
            }

            if let attachmentError, !attachmentError.isEmpty {
                HStack(spacing: 8) {
                    Image(systemName: "exclamationmark.triangle.fill")
                        .font(.caption)
                        .foregroundStyle(theme.textError)
                    Text(attachmentError)
                        .font(.caption)
                        .foregroundStyle(theme.foreground)
                        .lineLimit(2)
                    Spacer(minLength: 0)
                    Button {
                        onDismissAttachmentError()
                    } label: {
                        Image(systemName: "xmark")
                            .font(.caption2.weight(.semibold))
                            .foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                }
                .padding(.horizontal, 12)
                .padding(.vertical, 8)
                .background(theme.card, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            }

            if !pendingAttachments.isEmpty {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 8) {
                        ForEach(pendingAttachments) { item in
                            HStack(spacing: 6) {
                                Image(systemName: item.mime?.hasPrefix("image/") == true ? "photo" : "doc.fill")
                                    .font(.caption)
                                    .foregroundStyle(theme.textAccent)
                                Text(item.name)
                                    .font(.caption.weight(.medium))
                                    .foregroundStyle(theme.foreground)
                                    .lineLimit(1)
                                    .truncationMode(.middle)
                                Button {
                                    onRemoveAttachment(item.id)
                                } label: {
                                    Image(systemName: "xmark.circle.fill")
                                        .font(.caption)
                                        .foregroundStyle(theme.mutedForeground)
                                }
                                .buttonStyle(.plain)
                                .accessibilityLabel("Remove \(item.name)")
                            }
                            .padding(.horizontal, 10)
                            .padding(.vertical, 6)
                            .background(theme.card, in: Capsule())
                            .overlay(Capsule().strokeBorder(theme.border, lineWidth: 0.5))
                        }
                    }
                    .padding(.horizontal, 4)
                }
            }

            // Keep a single TextField and action button pair across compact/expanded
            // states so toggling focus never drops keyboard responder state.
            ZStack(alignment: .bottom) {
                textField
                    .padding(.leading, isExpanded ? 18 : 52)
                    .padding(.trailing, isExpanded ? 18 : 52)
                    .padding(.top, isExpanded ? 14 : 13)
                    .padding(.bottom, isExpanded ? 52 : 13)

                HStack(spacing: 10) {
                    attachmentsButton
                    if isExpanded {
                        permissionModeChip
                            .transition(.opacity.combined(with: .scale(scale: 0.9, anchor: .leading)))
                        personaChip
                            .transition(.opacity.combined(with: .scale(scale: 0.9, anchor: .leading)))
                        Spacer(minLength: 0)
                        if let usage, usage.turns > 0 {
                            usageRing(usage)
                                .transition(.opacity.combined(with: .scale(scale: 0.9)))
                        }
                        modelPickerChip
                            .transition(.opacity.combined(with: .scale(scale: 0.9, anchor: .trailing)))
                    } else {
                        Spacer(minLength: 0)
                            .allowsHitTesting(false)
                    }
                    sendOrCancelButton
                }
                .padding(.horizontal, isExpanded ? 14 : 10)
                .padding(.bottom, 8)
            }
            .background(
                glassShape
                    .fill(.clear)
                    .glassEffect(.regular.interactive(true), in: glassShape)
            )
            .animation(
                Theme.Motion.value(.spring(response: 0.3, dampingFraction: 1.0), reduced: reduceMotion),
                value: isExpanded
            )
        }
        .onAppear { localFocus = focused.wrappedValue }
        .onChange(of: focused.wrappedValue) { _, new in
            localFocus = new
        }
        .onChange(of: mentionDirectoryKey) { _, key in
            if let key, directoryListings[key] == nil, let info = mentionPathComponents {
                onRequestDirectory(info.resolvedDirPath)
            }
        }
        .sheet(isPresented: $showsModelPicker) {
            ModelPickerSheet(
                currentModel: model,
                availableModels: availableModels,
                thinkingVariant: thinkingVariant,
                onPickModel: onPickModel,
                onPickThinkingVariant: onPickThinkingVariant,
                onRefreshModels: onRefreshModels
            )
            .presentationDetents([.medium, .large])
            .presentationDragIndicator(.visible)
        }
    }

    @ViewBuilder
    private var mentionPopover: some View {
        let shape = RoundedRectangle(cornerRadius: 16, style: .continuous)
        VStack(spacing: 0) {
            ForEach(Array(mentionSuggestions.enumerated()), id: \.element.id) { idx, item in
                if idx > 0 {
                    Divider().opacity(0.35).padding(.horizontal, 12)
                }
                Button {
                    applyMention(item)
                } label: {
                    HStack(spacing: 10) {
                        Image(systemName: item.icon)
                            .font(.caption)
                            .foregroundStyle(.secondary)
                            .frame(width: 16)
                        Text(item.label)
                            .font(.mewMono(13))
                            .foregroundStyle(.primary)
                            .lineLimit(1)
                            .truncationMode(.middle)
                        if let detail = item.detail, !detail.isEmpty {
                            Text(detail)
                                .font(.caption)
                                .foregroundStyle(.secondary)
                                .lineLimit(1)
                                .truncationMode(.tail)
                        }
                        Spacer(minLength: 0)
                    }
                    .padding(.horizontal, 14)
                    .padding(.vertical, 9)
                    .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
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
        .shadow(color: .black.opacity(0.1), radius: 8, y: 3)
    }

    @ViewBuilder
    private var textField: some View {
        TextField(isBusy ? "Queue follow-up…" : "Message mew…", text: $text, axis: .vertical)
            .lineLimit(1...6)
            .focused($localFocus)
            .submitLabel(.send)
            .onSubmit(onSubmit)
    }

    @ViewBuilder
    private var attachmentsButton: some View {
        // + menu — browse workspace files, pick photos, or attach local files.
        Menu {
            Button("Browse workspace files", systemImage: "folder") {
                onAttachments()
            }
            Button("Photo library", systemImage: "photo.on.rectangle") {
                onPickPhotos()
            }
            Button("Choose file", systemImage: "doc.badge.plus") {
                onPickFiles()
            }
        } label: {
            Image(systemName: "plus")
                .font(.system(size: 18, weight: .regular))
                .foregroundStyle(.primary)
                .frame(width: Theme.Layout.compactControlSize, height: Theme.Layout.compactControlSize)
                .contentShape(Rectangle())
        }
        .accessibilityLabel("Attachments and files")
    }

    @ViewBuilder
    private var modelPickerChip: some View {
        Button {
            showsModelPicker = true
        } label: {
            modelPickerLabel
        }
        .buttonStyle(MewPressButtonStyle())
        .accessibilityLabel("Select model and thinking level")
        .accessibilityValue(modelAccessibilityValue)
    }

    private var modelAccessibilityValue: String {
        let base = model?.displayName ?? model?.model ?? "Model"
        if let badge = activeThinkingVariant {
            return "\(base), \(badge)"
        }
        return base
    }

    private var activeThinkingVariant: String? {
        guard let v = thinkingVariant?.trimmingCharacters(in: .whitespacesAndNewlines),
              !v.isEmpty, v != "none" else {
            return nil
        }
        return v
    }

    @ViewBuilder
    private var modelPickerLabel: some View {
        ZStack(alignment: .bottomTrailing) {
            ProviderIconView(provider: model?.provider ?? "", size: 20)
                .foregroundStyle(.primary)

            if let variant = activeThinkingVariant {
                ThinkingGauge(variant: variant, size: 13, tint: theme.textAccent)
                    .padding(1.5)
                    .background(Circle().fill(theme.card))
                    .offset(x: 5, y: 5)
            }
        }
        .frame(width: Theme.Layout.compactControlSize, height: Theme.Layout.compactControlSize)
        .contentShape(Rectangle())
    }

    @ViewBuilder
    private var permissionModeChip: some View {
        let modes = ["standard", "permissive", "auto", "auto_plus", "dangerous"]
        let labels = ["Standard", "Permissive", "Auto", "Auto+", "Dangerous!"]
        Menu {
            ForEach(Array(modes.enumerated()), id: \.offset) { idx, mode in
                Button {
                    onPickPermissionMode(mode)
                } label: {
                    HStack {
                        Text(labels[idx])
                        if permissionMode == mode {
                            Image(systemName: "checkmark")
                        }
                    }
                }
            }
        } label: {
            Image(systemName: permissionModeIcon)
                .font(.system(size: 16))
                .foregroundStyle(permissionModeColor)
                .frame(width: Theme.Layout.compactControlSize, height: Theme.Layout.compactControlSize)
                .contentShape(Rectangle())
        }
        .accessibilityLabel("Permission mode")
    }

    private var permissionModeIcon: String {
        let mode = permissionMode ?? "standard"
        return mode == "standard" ? "shield" : "shield.fill"
    }

    private var permissionModeColor: Color {
        switch permissionMode ?? "standard" {
        case "dangerous":
            return .red
        case "permissive", "auto", "auto_plus":
            return .orange
        default:
            return .secondary
        }
    }

    @ViewBuilder
    private var personaChip: some View {
        if !availablePersonas.isEmpty {
            Menu {
                Button {
                    onPickPersona("default")
                } label: {
                    HStack {
                        Text("Default")
                        if currentPersona == nil {
                            Image(systemName: "checkmark")
                        }
                    }
                }
                ForEach(availablePersonas, id: \.name) { persona in
                    Button {
                        onPickPersona(persona.name)
                    } label: {
                        HStack {
                            Text(persona.name)
                            if !persona.description.isEmpty {
                                Text(persona.description).font(.caption).foregroundStyle(.secondary)
                            }
                            if currentPersona == persona.name {
                                Image(systemName: "checkmark")
                            }
                        }
                    }
                }
            } label: {
                Image(systemName: currentPersona != nil ? "person.crop.circle.fill" : "person.crop.circle")
                    .font(.system(size: 16))
                    .foregroundStyle(currentPersona != nil ? theme.textAccent : Color.secondary)
                    .frame(width: Theme.Layout.compactControlSize, height: Theme.Layout.compactControlSize)
                    .contentShape(Rectangle())
            }
            .accessibilityLabel("Persona")
        }
    }

    @ViewBuilder
    private func usageRing(_ usage: SessionUsage) -> some View {
        let contextFraction: Double = {
            guard let ctx = model?.contextWindow, ctx > 0, usage.inputTokens > 0 else { return 0 }
            return min(1.0, max(0.05, Double(usage.inputTokens) / Double(ctx)))
        }()
        let pctText: String? = {
            guard let ctx = model?.contextWindow, ctx > 0, usage.inputTokens > 0 else { return nil }
            let pct = min(100, Int(Double(usage.inputTokens) / Double(ctx) * 100))
            return "\(pct)% context used"
        }()

        Menu {
            if let pctText {
                Text(pctText)
            }
            Text(String(format: "$%.4f", usage.cost))
            Text("\(usage.turns) turn\(usage.turns == 1 ? "" : "s")")
        } label: {
            ZStack {
                Circle()
                    .stroke(theme.mutedForeground.opacity(0.32), lineWidth: 2.2)
                if contextFraction > 0 {
                    Circle()
                        .trim(from: 0, to: contextFraction)
                        .stroke(
                            contextFraction > 0.85 ? theme.textError : theme.foreground,
                            style: StrokeStyle(lineWidth: 2.2, lineCap: .round)
                        )
                        .rotationEffect(.degrees(-90))
                }
            }
            .frame(width: 18, height: 18)
            .frame(width: Theme.Layout.compactControlSize, height: Theme.Layout.compactControlSize)
            .contentShape(Rectangle())
        }
        .accessibilityLabel("Session context and usage")
        .accessibilityValue(pctText ?? String(format: "$%.4f", usage.cost))
    }

    @ViewBuilder
    private var sendOrCancelButton: some View {
        if isBusy {
            HStack(spacing: 6) {
                if canEnqueue {
                    Button {
                        onSteer()
                    } label: {
                        Text("Steer")
                            .font(.caption.weight(.semibold))
                            .foregroundStyle(.orange)
                            .padding(.horizontal, 10)
                            .frame(height: Theme.Layout.compactControlSize)
                            .background(Capsule().fill(Color.orange.opacity(0.16)))
                    }
                    .accessibilityLabel("Steer current turn")
                    .buttonStyle(MewPressButtonStyle())

                    Button {
                        onSubmit()
                    } label: {
                        Image(systemName: "arrow.up.circle.fill")
                            .font(.system(size: 28))
                    }
                    .accessibilityLabel("Queue follow-up")
                    .buttonStyle(MewPressButtonStyle())
                }
                if isStreaming {
                    Button {
                        onCancel()
                    } label: {
                        Image(systemName: "stop.circle.fill")
                            .font(.system(size: 28))
                            .foregroundStyle(.red)
                    }
                    .accessibilityLabel("Stop generating")
                    .buttonStyle(MewPressButtonStyle())
                }
            }
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
            .buttonStyle(MewPressButtonStyle())
        }
    }

}

// MARK: - Provider Icon & Model Picker Sheet

private struct ProviderIconView: View {
    let provider: String
    var size: CGFloat = 14

    private var assetName: String? {
        switch provider.lowercased() {
        case "antigravity", "google", "gemini":
            return "provider-antigravity"
        case "codex", "openai":
            return "provider-codex"
        case "deepseek":
            return "provider-deepseek"
        case "kimi-for-coding", "kimi":
            return "provider-kimi"
        case "alibaba-token-plan", "alibaba-token-plan-cn":
            return "provider-alibabacloud"
        case "opencode-zen", "opencode-go":
            return "provider-opencode"
        case "z-ai", "zai":
            return "provider-zai"
        default:
            return nil
        }
    }

    var body: some View {
        Group {
            if let assetName {
                Image(assetName)
                    .renderingMode(.template)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .frame(width: size, height: size)
            } else {
                Image(systemName: "cpu")
                    .font(.system(size: size * 0.85, weight: .medium))
                    .frame(width: size, height: size)
            }
        }
        .accessibilityHidden(true)
    }
}

private struct ModelPickerSheet: View {
    let currentModel: ModelSummary?
    let availableModels: [ModelSummary]
    let thinkingVariant: String?
    let onPickModel: (ModelSummary) -> Void
    let onPickThinkingVariant: (String) -> Void
    let onRefreshModels: () -> Void

    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme
    @State private var searchText = ""

    private var filteredModels: [ModelSummary] {
        let q = searchText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !q.isEmpty else { return availableModels }
        return availableModels.filter { m in
            (m.displayName?.localizedCaseInsensitiveContains(q) == true)
                || m.model.localizedCaseInsensitiveContains(q)
                || m.provider.localizedCaseInsensitiveContains(q)
                || (m.description?.localizedCaseInsensitiveContains(q) == true)
        }
    }

    var body: some View {
        NavigationStack {
            VStack(spacing: 0) {
                if let currentModel, !currentModel.thinkingVariants.isEmpty {
                    thinkingCard(for: currentModel)
                        .padding(.horizontal, 16)
                        .padding(.top, 8)
                        .padding(.bottom, 10)
                }

                List {
                    if filteredModels.isEmpty {
                        ContentUnavailableView(
                            availableModels.isEmpty ? "No models loaded" : "No matching models",
                            systemImage: "cpu",
                            description: Text(availableModels.isEmpty ? "Tap refresh to load models from the daemon." : "Try a provider or model name.")
                        )
                        .listRowSeparator(.hidden)
                        .listRowBackground(Color.clear)
                    } else {
                        ForEach(filteredModels, id: \.id) { m in
                            Button {
                                onPickModel(m)
                                dismiss()
                            } label: {
                                modelRowContent(m)
                            }
                            .buttonStyle(.plain)
                            .listRowBackground(Color.clear)
                        }
                    }
                }
                .listStyle(.plain)
                .mewThemedPlainList(theme)
                .searchable(text: $searchText, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search models")
            }
            .background(theme.background.ignoresSafeArea())
            .navigationTitle("Models")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .topBarLeading) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Refresh models", systemImage: "arrow.clockwise") {
                        onRefreshModels()
                    }
                }
            }
        }
    }

    @ViewBuilder
    private func thinkingCard(for model: ModelSummary) -> some View {
        let stops = ["none"] + model.thinkingVariants
        let current = thinkingVariant ?? "none"
        let activeLabel = current == "none" ? "Off" : current.capitalized
        let shape = RoundedRectangle(cornerRadius: 16, style: .continuous)

        VStack(spacing: 10) {
            HStack(spacing: 12) {
                ThinkingGauge(variant: current, size: 52, tint: theme.textAccent, showsTicks: true)
                VStack(alignment: .leading, spacing: 2) {
                    Text(activeLabel)
                        .font(.headline)
                        .foregroundStyle(theme.foreground)
                    Text(shortName(model))
                        .font(.caption)
                        .foregroundStyle(theme.mutedForeground)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
            }

            ThinkingEffortSlider(
                stops: stops,
                selection: current,
                onSelect: onPickThinkingVariant
            )
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
        .background(theme.card, in: shape)
        .overlay(shape.strokeBorder(theme.border, lineWidth: 0.5))
    }

    @ViewBuilder
    private func modelRowContent(_ m: ModelSummary) -> some View {
        let selected = currentModel?.id == m.id
        HStack(spacing: 12) {
            ProviderIconView(provider: m.provider, size: 16)
            VStack(alignment: .leading, spacing: 2) {
                Text(shortName(m))
                    .font(.body.weight(selected ? .semibold : .regular))
                    .foregroundStyle(theme.foreground)
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Text(m.provider)
                    if let window = m.contextWindow, window > 0 {
                        Text("·")
                        Text(formatContext(window))
                            .monospacedDigit()
                    }
                }
                .font(.caption)
                .foregroundStyle(theme.mutedForeground)
                if let desc = m.description, !desc.isEmpty {
                    Text(desc)
                        .font(.caption2)
                        .foregroundStyle(theme.mutedForeground)
                        .lineLimit(1)
                }
            }
            Spacer(minLength: 8)
            if selected {
                Image(systemName: "checkmark")
                    .font(.subheadline.weight(.semibold))
                    .foregroundStyle(theme.textAccent)
            }
        }
        .padding(.vertical, 4)
        .contentShape(Rectangle())
    }

    private func shortName(_ m: ModelSummary) -> String {
        if let display = m.displayName?.trimmingCharacters(in: .whitespacesAndNewlines), !display.isEmpty {
            return display
        }
        return m.model.split(separator: "/").last.map(String.init) ?? m.model
    }

    private func formatContext(_ bytes: Int64) -> String {
        let kb = Double(bytes) / 1024
        if kb >= 1024 {
            return String(format: "%.0fK ctx", kb / 1024)
        }
        return String(format: "%.0fK ctx", kb)
    }
}

private struct ThinkingEffortSlider: View {
    let stops: [String]
    let selection: String
    let onSelect: (String) -> Void

    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.mewTheme) private var theme
    @State private var dragIndex: Int?

    private var activeIndex: Int {
        if let dragIndex { return dragIndex }
        return stops.firstIndex(of: selection) ?? 0
    }

    var body: some View {
        GeometryReader { geo in
            let trackHeight: CGFloat = 36
            let thumbSize: CGFloat = 28
            let pad: CGFloat = 4
            let usableWidth = max(1, geo.size.width - thumbSize - pad * 2)
            let count = max(2, stops.count)
            let fraction = CGFloat(activeIndex) / CGFloat(count - 1)
            let thumbX = pad + fraction * usableWidth
            let fillWidth = thumbX + thumbSize - pad

            ZStack(alignment: .leading) {
                Capsule()
                    .fill(theme.muted)
                    .overlay(Capsule().strokeBorder(theme.border, lineWidth: 0.5))

                Capsule()
                    .fill(theme.foreground.opacity(activeIndex == 0 ? 0.25 : 0.85))
                    .frame(width: max(thumbSize, fillWidth), height: trackHeight - pad * 2)
                    .padding(.leading, pad)

                // Discrete stop markers
                ForEach(0..<stops.count, id: \.self) { idx in
                    let x = pad + (CGFloat(idx) / CGFloat(count - 1)) * usableWidth + thumbSize / 2
                    Circle()
                        .fill(idx <= activeIndex ? theme.background.opacity(0.45) : theme.mutedForeground.opacity(0.45))
                        .frame(width: 5, height: 5)
                        .position(x: x, y: trackHeight / 2)
                }

                // Thumb
                Circle()
                    .fill(theme.foreground)
                    .shadow(color: .black.opacity(0.22), radius: 3, y: 1)
                    .frame(width: thumbSize, height: thumbSize)
                    .offset(x: thumbX)
            }
            .frame(height: trackHeight)
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0)
                    .onChanged { value in
                        let clamped = min(max(0, value.location.x - pad - thumbSize / 2), usableWidth)
                        let idx = Int((clamped / usableWidth * CGFloat(count - 1)).rounded())
                        let bounded = min(max(0, idx), stops.count - 1)
                        if bounded != activeIndex {
                            UISelectionFeedbackGenerator().selectionChanged()
                            withAnimation(Theme.Motion.value(.spring(response: 0.25, dampingFraction: 1.0), reduced: reduceMotion)) {
                                dragIndex = bounded
                            }
                            onSelect(stops[bounded])
                        } else if dragIndex == nil {
                            dragIndex = bounded
                        }
                    }
                    .onEnded { _ in
                        dragIndex = nil
                    }
            )
            .animation(Theme.Motion.value(.spring(response: 0.25, dampingFraction: 1.0), reduced: reduceMotion), value: activeIndex)
        }
        .frame(height: 36)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel("Thinking effort")
        .accessibilityValue(stops[activeIndex] == "none" ? "Off" : stops[activeIndex].capitalized)
    }
}

// MARK: - FileBrowserSheet

private struct FileBrowserItem: Identifiable {
    let path: String
    var id: String { path }
}

/// A modal directory browser. Lists entries for `currentPath`; tapping a
/// folder navigates in (calls `onNavigate` with the new path), tapping a
/// file opens a preview sheet, and the trailing `+` inserts `@path` directly.
struct FileBrowserSheet: View {
    let sessionId: String
    let currentPath: String
    let onNavigate: (String) -> Void
    let onPick: (String) -> Void
    let onClose: () -> Void

    @EnvironmentObject private var store: AppStore
    @Environment(\.mewTheme) private var theme
    @State private var previewingPath: String?

    private var directoryKey: String {
        "\(sessionId)::\(currentPath)"
    }

    private var listing: DirListing? {
        store.directoryListings[directoryKey]
    }

    private var isLoading: Bool {
        store.directoryLoading.contains(directoryKey)
    }

    var body: some View {
        NavigationStack {
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(theme.background.ignoresSafeArea())
                .navigationTitle(currentPath.isEmpty ? "Files" : (currentPath as NSString).lastPathComponent)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarTrailing) {
                        Button("Done", action: onClose)
                    }
                    ToolbarItem(placement: .topBarLeading) {
                        if !currentPath.isEmpty {
                            Button {
                                onNavigate(parentPath(of: currentPath))
                            } label: {
                                Image(systemName: "chevron.left")
                            }
                        }
                    }
                }
        }
        .task(id: currentPath) {
            // Fetch on open and on every navigation. Idempotent on the
            // daemon side.
            store.fetchDirectory(sessionId: sessionId, path: currentPath.isEmpty ? nil : currentPath)
        }
        .sheet(item: Binding(
            get: { previewingPath.map { FileBrowserItem(path: $0) } },
            set: { previewingPath = $0?.path }
        )) { item in
            FilePreviewSheet(
                sessionId: sessionId,
                path: item.path,
                onInsert: {
                    previewingPath = nil
                    onPick(item.path)
                }
            )
        }
    }

    @ViewBuilder
    private var content: some View {
        if isLoading && listing == nil {
            VStack {
                Spacer()
                ProgressView("Loading…")
                Spacer()
            }
        } else if let listing {
            List {
                Section {
                    if let entries = listing.entries.isEmpty ? nil : listing.entries {
                        ForEach(entries, id: \.name) { entry in
                            let fullPath = joinPath(currentPath, entry.name)
                            HStack(spacing: 8) {
                                Button {
                                    if entry.isDir {
                                        onNavigate(fullPath)
                                    } else {
                                        previewingPath = fullPath
                                    }
                                } label: {
                                    row(entry)
                                }
                                .buttonStyle(.plain)

                                if !entry.isDir {
                                    Button {
                                        onPick(fullPath)
                                    } label: {
                                        Image(systemName: "plus.circle")
                                            .font(.body)
                                            .foregroundStyle(theme.textAccent)
                                            .frame(minWidth: 32, minHeight: 32)
                                    }
                                    .buttonStyle(.plain)
                                    .accessibilityLabel("Insert @\(entry.name)")
                                }
                            }
                        }
                    } else {
                        Text("Empty directory")
                            .font(.callout)
                            .foregroundStyle(.secondary)
                    }
                } header: {
                    Text(currentPath)
                        .font(.mewMono(12))
                        .foregroundStyle(.secondary)
                        .textCase(nil)
                }
                .listRowBackground(Color.clear)
            }
            .listStyle(.plain)
            .mewThemedPlainList(theme)
        } else {
            VStack {
                Spacer()
                Image(systemName: "folder")
                    .font(.system(size: 48))
                    .foregroundStyle(.tertiary)
                Text("Couldn't list directory")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                Button("Retry") {
                    store.fetchDirectory(sessionId: sessionId, path: currentPath.isEmpty ? nil : currentPath)
                }
                .buttonStyle(.borderedProminent)
                Spacer()
            }
        }
    }

    @ViewBuilder
    private func row(_ entry: DirEntry) -> some View {
        HStack(spacing: 10) {
            Image(systemName: entry.isDir ? "folder.fill" : "doc.fill")
                .font(.callout)
                .foregroundStyle(entry.isDir ? Color.accentColor : Color.secondary)
                .frame(width: 22)
            VStack(alignment: .leading, spacing: 2) {
                Text(entry.name)
                    .font(.body)
                    .foregroundStyle(.primary)
                if let size = entry.size {
                    Text(formatSize(size))
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
            Spacer()
        }
        .contentShape(Rectangle())
    }

    private func parentPath(of path: String) -> String {
        (path as NSString).deletingLastPathComponent
    }

    private func joinPath(_ base: String, _ component: String) -> String {
        if base.isEmpty { return component }
        return (base as NSString).appendingPathComponent(component)
    }

    private func formatSize(_ bytes: UInt64) -> String {
        let f = ByteCountFormatter()
        f.countStyle = .file
        return f.string(fromByteCount: Int64(bytes))
    }
}

private struct FilePreviewSheet: View {
    let sessionId: String
    let path: String
    var onInsert: (() -> Void)? = nil

    @EnvironmentObject private var store: AppStore
    @Environment(\.dismiss) private var dismiss
    @Environment(\.mewTheme) private var theme

    private var previewState: FilePreviewState {
        store.filePreviews[store.filePreviewKey(sessionId: sessionId, path: path)] ?? .loading
    }

    var body: some View {
        NavigationStack {
            content
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .background(theme.background.ignoresSafeArea())
                .navigationTitle((path as NSString).lastPathComponent)
                .navigationBarTitleDisplayMode(.inline)
                .toolbar {
                    ToolbarItem(placement: .topBarLeading) {
                        Button("Close") { dismiss() }
                    }
                    ToolbarItem(placement: .topBarTrailing) {
                        if let onInsert {
                            Button("Insert @path", action: onInsert)
                        }
                    }
                }
        }
        .task(id: path) {
            store.readFilePreview(sessionId: sessionId, path: path)
        }
    }

    @ViewBuilder
    private var content: some View {
        switch previewState {
        case .loading:
            ProgressView("Loading file…")
        case .failed(let error):
            ContentUnavailableView {
                Label("Couldn't preview file", systemImage: "doc.questionmark")
            } description: {
                Text(error)
            } actions: {
                Button("Retry") {
                    store.readFilePreview(sessionId: sessionId, path: path)
                }
                .buttonStyle(.borderedProminent)
            }
        case .loaded(let text, let truncated, let language):
            ScrollView([.vertical, .horizontal]) {
                VStack(alignment: .leading, spacing: 10) {
                    if language != nil || truncated {
                        HStack(spacing: 8) {
                            if let language, !language.isEmpty {
                                Text(language)
                                    .font(.caption.weight(.medium))
                                    .foregroundStyle(theme.mutedForeground)
                                    .padding(.horizontal, 8)
                                    .padding(.vertical, 3)
                                    .background(theme.card, in: Capsule())
                            }
                            if truncated {
                                Text("Preview truncated")
                                    .font(.caption)
                                    .foregroundStyle(theme.textAccent)
                            }
                        }
                    }

                    Text(text.isEmpty ? "(Empty file)" : text)
                        .font(.mewMono(12))
                        .foregroundStyle(text.isEmpty ? theme.mutedForeground : theme.foreground)
                        .textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                }
                .padding(16)
            }
        }
    }
}
