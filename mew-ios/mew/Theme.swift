import SwiftUI

/// App theme colors — a dark-friendly palette matching mew's web UI.
enum Theme {
    enum Layout {
        static let panelRadius: CGFloat = 16
        static let controlRadius: CGFloat = 12
        static let compactControlSize: CGFloat = 36
    }

    enum Motion {
        static let press = Animation.snappy(duration: 0.12, extraBounce: 0)
        static let disclosure = Animation.snappy(duration: 0.2, extraBounce: 0)
        static let surface = Animation.spring(response: 0.3, dampingFraction: 0.9)

        static func value(_ animation: Animation, reduced: Bool) -> Animation? {
            reduced ? nil : animation
        }
    }

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

/// Small custom controls should acknowledge touch-down without fighting the
/// platform's own navigation and list gestures.
struct MewPressButtonStyle: ButtonStyle {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    func makeBody(configuration: Configuration) -> some View {
        configuration.label
            .scaleEffect(configuration.isPressed && !reduceMotion ? 0.97 : 1)
            .animation(
                Theme.Motion.value(Theme.Motion.press, reduced: reduceMotion),
                value: configuration.isPressed
            )
    }
}
