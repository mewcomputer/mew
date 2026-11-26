import SwiftUI

/// Custom fonts bundled with the app, matching the mew web UI.
extension Font {
    // Instrument Sans — the primary sans-serif body font
    static let mewSans = Font.custom("InstrumentSans-Regular", size: 17, relativeTo: .body)
    static let mewSansMedium = Font.custom("InstrumentSans-Regular", size: 17, relativeTo: .body).weight(.medium)
    static let mewSansSemibold = Font.custom("InstrumentSans-Regular", size: 17, relativeTo: .body).weight(.semibold)
    static let mewSansBold = Font.custom("InstrumentSans-Regular", size: 17, relativeTo: .body).weight(.bold)

    // Ioskeley Mono — code/monospaced font (Regular and Medium are separate faces)
    static let mewMono = Font.custom("Ioskeley-Mono", size: 17, relativeTo: .body)
    static let mewMonoMedium = Font.custom("Ioskeley-Mono-Medium", size: 17, relativeTo: .body)

    /// Convenience: Instrument Sans at a specific size
    static func mewSans(_ size: CGFloat) -> Font {
        Font.custom("InstrumentSans-Regular", size: size, relativeTo: .body)
    }

    /// Convenience: Ioskeley Mono at a specific size
    static func mewMono(_ size: CGFloat) -> Font {
        Font.custom("Ioskeley-Mono", size: size, relativeTo: .body)
    }
}

/// Check if custom fonts are available at runtime.
enum MewFonts {
    static var sansAvailable: Bool {
        UIFont(name: "InstrumentSans-Regular", size: 12) != nil
    }
    static var monoAvailable: Bool {
        UIFont(name: "Ioskeley-Mono", size: 12) != nil
    }

    /// Log any custom fonts that failed to register. A miss means the
    /// PostScript name here disagrees with the font's `name` table or the
    /// file is missing from `UIAppFonts` — the app silently falls back to a
    /// system font otherwise, so surface it loudly in debug builds.
    static func verify() {
        let checks: [(String, Bool)] = [
            ("InstrumentSans-Regular (sans)", sansAvailable),
            ("Ioskeley-Mono (mono)", monoAvailable),
            ("Ioskeley-Mono-Medium (mono medium)", UIFont(name: "Ioskeley-Mono-Medium", size: 12) != nil),
        ]
        let missing = checks.filter { !$0.1 }.map(\.0)
        if missing.isEmpty {
            print("[MewFonts] all custom fonts registered")
        } else {
            print("[MewFonts] MISSING (falling back to system): \(missing.joined(separator: ", "))")
            assertionFailure("Custom fonts failed to register: \(missing.joined(separator: ", "))")
        }
    }
}
