import SwiftUI
import Lottie

/// Mascot animation states backed by the Lottie files bundled from `mew-web-ui/public/mascot/status/`.
enum MascotState: String {
    case thinking
    case working
    case searching
    case waiting
    case readingFiles = "reading-files"
    case editingCode = "editing-code"
    case shellCommand = "shell-command"
    case runningTests = "running-tests"
    case retrying
    case python
}

/// Renders the animated mew cat mascot using the dark-ink or light-ink Lottie
/// asset matching the active color scheme, falling back to a static image
/// when Reduce Motion is enabled.
struct MascotView: View {
    let state: MascotState
    var size: CGFloat = 28

    @Environment(\.colorScheme) private var colorScheme
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var inkDirectory: String {
        colorScheme == .dark ? "light-ink" : "dark-ink"
    }

    private var jsonURL: URL? {
        Bundle.main.url(forResource: state.rawValue, withExtension: "json", subdirectory: "Mascot/\(inkDirectory)")
    }

    private var neutralImage: UIImage? {
        guard let url = Bundle.main.url(forResource: "neutral", withExtension: "png", subdirectory: "Mascot/\(inkDirectory)") else {
            return nil
        }
        return UIImage(contentsOfFile: url.path)
    }

    var body: some View {
        Group {
            if !reduceMotion, let url = jsonURL {
                LottieView(animation: .filepath(url.path))
                    .playing(loopMode: .loop)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
            } else if let neutralImage {
                Image(uiImage: neutralImage)
                    .resizable()
                    .aspectRatio(contentMode: .fit)
            } else {
                Image(systemName: "cat.fill")
                    .resizable()
                    .aspectRatio(contentMode: .fit)
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}
