import SwiftUI

/// Refresh feedback shown alongside the last successfully loaded content.
struct RefreshStatusView: View {
    let isLoading: Bool
    let error: String?
    let retry: () -> Void

    var body: some View {
        if isLoading || error != nil {
            VStack(alignment: .leading, spacing: 8) {
                if isLoading {
                    HStack(spacing: 8) {
                        ProgressView()
                        Text("Refreshing…").foregroundStyle(.secondary)
                    }
                } else if let error {
                    Label("Couldn't refresh", systemImage: "exclamationmark.triangle")
                        .fontWeight(.semibold)
                    Text("Showing the previous result. \(error)").foregroundStyle(.secondary)
                    Button("Try again", action: retry)
                        .buttonStyle(.bordered)
                        .controlSize(.large)
                }
            }
            .font(.callout)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding()
            .background(.bar)
        }
    }
}
