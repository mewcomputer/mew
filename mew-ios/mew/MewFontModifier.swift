import SwiftUI

/// Convenience extensions for applying mew's default typography in SwiftUI views.
extension View {
    /// Apply Instrument Sans as the body font for this subtree.
    func mewBodyFont() -> some View {
        self.font(.mewSans)
    }

    /// Apply Ioskeley Mono for code-like content.
    func mewMonoFont() -> some View {
        self.font(.mewMono)
    }
}
