import SwiftUI

/// Thinking effort drawn as a speedometer: a dial that sweeps from rest to the
/// top of the scale, with the needle where the current level sits.
///
/// Providers name their levels differently and offer different subsets, so the
/// needle reads a fixed canonical order rather than "stop 3 of 5". The dial is
/// deliberately absolute: switching from a model that tops out at `medium` to
/// one that offers `ultra` moves the needle, which is the point of the gauge.
struct ThinkingGauge: View {
    /// Canonical effort ladder: the catalog's `none`..`ultra` order with the
    /// providers' optional `minimal` slotted in below `low`.
    static let scale = ["none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra"]

    var variant: String?
    var size: CGFloat
    var tint: Color
    /// Tick marks at every named level. Only worth drawing on the large dial.
    var showsTicks = false

    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    /// Needle position in 0...1. `none` rests at the lower left, and a level this
    /// build does not know sits at the top so an engaged setting never looks
    /// idle.
    static func position(for variant: String?) -> Double {
        let level = variant?.trimmingCharacters(in: .whitespacesAndNewlines).lowercased() ?? ""
        if level.isEmpty || level == "off" { return 0 }
        guard let index = scale.firstIndex(of: level) else { return 1 }
        return Double(index) / Double(scale.count - 1)
    }

    private var position: Double { Self.position(for: variant) }

    // A speedometer sweeps 240 degrees clockwise from the lower left to the
    // lower right, leaving the bottom open for the needle hub.
    private let startAngle = 150.0
    private let sweep = 240.0

    /// Screen angle in degrees of a position on the dial. 0 is 3 o'clock and the
    /// dial runs clockwise, so the arc, the ticks, and the needle all agree.
    private func angle(_ fraction: Double) -> Double { startAngle + sweep * fraction }

    var body: some View {
        let stroke = max(1.2, size * 0.09)
        let radius = (size - stroke) / 2
        let end = max(0.001, position)

        ZStack {
            Circle()
                .trim(from: 0, to: sweep / 360)
                .rotation(.degrees(startAngle))
                .stroke(tint.opacity(0.22), style: StrokeStyle(lineWidth: stroke, lineCap: .round))

            Circle()
                .trim(from: 0, to: sweep / 360 * end)
                .rotation(.degrees(startAngle))
                .stroke(tint, style: StrokeStyle(lineWidth: stroke, lineCap: .round))

            if showsTicks {
                ForEach(Self.scale.indices, id: \.self) { index in
                    let fraction = Double(index) / Double(Self.scale.count - 1)
                    Capsule()
                        .fill(tint.opacity(index <= filledTicks ? 0.9 : 0.35))
                        .frame(width: max(1, size * 0.02), height: max(2, size * 0.07))
                        .offset(y: -radius * 0.8)
                        // The capsule starts at 12 o'clock, hence the -270.
                        .rotationEffect(.degrees(angle(fraction) - 270))
                }
            }

            Capsule()
                .fill(tint.opacity(0.85))
                .frame(width: radius * 0.66, height: max(1.2, size * 0.055))
                .offset(x: radius * 0.33)
                .rotationEffect(.degrees(angle(position)))
                // A needle is a physical pointer, so it carries a little momentum
                // when the level changes instead of sliding.
                .animation(Theme.Motion.value(.spring(response: 0.4, dampingFraction: 0.8), reduced: reduceMotion), value: position)

            Circle()
                .fill(tint)
                .frame(width: max(2, size * 0.14), height: max(2, size * 0.14))
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    private var filledTicks: Int {
        Int((position * Double(Self.scale.count - 1)).rounded())
    }
}
