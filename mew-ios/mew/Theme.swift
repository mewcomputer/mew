import SwiftUI

/// App theme colors — a dark-friendly palette matching mew's web UI.
enum Theme {
    static let background = Color(.systemGroupedBackground)
    static let secondaryBackground = Color(.secondarySystemGroupedBackground)

    static let userBubble = Color.accentColor.opacity(0.15)
    static let userBubbleText = Color.primary

    static let assistantBubble = Color.clear
    static let assistantBubbleText = Color.primary

    static let reasoningText = Color.secondary
    static let toolPending = Color.gray
    static let toolRunning = Color.accentColor
    static let toolCompleted = Color.green
    static let toolError = Color.red

    static let permissionBg = Color.yellow.opacity(0.1)
    static let permissionBorder = Color.yellow.opacity(0.5)
    static let allowOnce = Color.green
    static let deny = Color.red

    static let needsAttention = Color.orange
    static let connected = Color.green
    static let connecting = Color.yellow
    static let disconnected = Color.gray
    static let backoff = Color.orange
}
