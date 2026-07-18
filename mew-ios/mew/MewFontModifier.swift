import SwiftUI
import UIKit

/// Applies MiSans as the default body font app-wide, with Banga for
/// the "mew" wordmark and Ioskeley Mono for code/monospaced text.
///
/// Usage: call `MewFonts.apply()` once at launch (in App.init or onAppear).
enum MewFontConfig {
    /// Call once at launch to set the default font for all UIKit-backed text
    /// (List rows, Form fields, navigation bars, etc.)
    static func apply() {
        // Set the default label font — affects most SwiftUI text
        if MewFonts.sansAvailable {
            UILabel.appearance().font = UIFont(name: "MiSansLatinVF", size: UIFont.systemFontSize)
        }
    }
}

/// Convenience extensions for use in SwiftUI views.
extension View {
    /// Apply MiSans as the body font for this subtree.
    func mewBodyFont() -> some View {
        self.font(.mewSans)
    }

    /// Apply Ioskeley Mono for code-like content.
    func mewMonoFont() -> some View {
        self.font(.mewMono)
    }
}
