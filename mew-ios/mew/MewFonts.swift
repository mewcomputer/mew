import SwiftUI

/// Custom fonts bundled with the app, matching the mew web UI.
extension Font {
    // MiSans — the primary sans-serif body font
    static let mewSans = Font.custom("MiSansLatinVF", size: UIFont.systemFontSize)
    static let mewSansMedium = Font.custom("MiSansLatinVF", size: UIFont.systemFontSize).weight(.medium)
    static let mewSansSemibold = Font.custom("MiSansLatinVF", size: UIFont.systemFontSize).weight(.semibold)
    static let mewSansBold = Font.custom("MiSansLatinVF", size: UIFont.systemFontSize).weight(.bold)

    // Banga — display/headline font
    static func mewDisplay(_ size: CGFloat) -> Font {
        Font.custom("Banga-ExtraLight", size: size)
    }

    // Ioskeley Mono — code/monospaced font (Regular and Medium are separate faces)
    static let mewMono = Font.custom("Ioskeley-Mono", size: UIFont.systemFontSize)
    static let mewMonoMedium = Font.custom("Ioskeley-Mono-Medium", size: UIFont.systemFontSize)

    // Junicode — serif body font
    static func mewSerif(_ size: CGFloat) -> Font {
        Font.custom("JunicodeVF-Regular", size: size)
    }
    static func mewSerifItalic(_ size: CGFloat) -> Font {
        Font.custom("JunicodeVF-Italic", size: size)
    }

    // OFL Goudy — display/headline serif font
    static func mewGoudy(_ size: CGFloat) -> Font {
        Font.custom("OFLGoudyStMTT", size: size)
    }
    static func mewGoudyItalic(_ size: CGFloat) -> Font {
        Font.custom("OFLGoudyStMTT-Italic", size: size)
    }

    // Garamontio — secondary serif font (regular and italic)
    static func mewGaramontio(_ size: CGFloat) -> Font {
        Font.custom("Garamontio-Regular", size: size)
    }
    static func mewGaramontioItalic(_ size: CGFloat) -> Font {
        Font.custom("Garamontio-Italic", size: size)
    }

    /// Convenience: mewSans at a specific size
    static func mewSans(_ size: CGFloat) -> Font {
        Font.custom("MiSansLatinVF", size: size)
    }

    /// Convenience: IoskeleyMono at a specific size
    static func mewMono(_ size: CGFloat) -> Font {
        Font.custom("Ioskeley-Mono", size: size)
    }
}

/// Check if custom fonts are available at runtime.
enum MewFonts {
    static var sansAvailable: Bool {
        UIFont(name: "MiSansLatinVF", size: 12) != nil
    }
    static var monoAvailable: Bool {
        UIFont(name: "Ioskeley-Mono", size: 12) != nil
    }
    static var displayAvailable: Bool {
        UIFont(name: "Banga-ExtraLight", size: 12) != nil
    }
    static var serifAvailable: Bool {
        UIFont(name: "JunicodeVF-Regular", size: 12) != nil
    }
    static var goudyAvailable: Bool {
        UIFont(name: "OFLGoudyStMTT", size: 12) != nil
    }
    static var garamontioAvailable: Bool {
        UIFont(name: "Garamontio-Regular", size: 12) != nil
    }

    /// Log any custom fonts that failed to register. A miss means the
    /// PostScript name here disagrees with the font's `name` table or the
    /// file is missing from `UIAppFonts` — the app silently falls back to a
    /// system font otherwise, so surface it loudly in debug builds.
    static func verify() {
        let checks: [(String, Bool)] = [
            ("MiSansLatinVF (sans)", sansAvailable),
            ("Ioskeley-Mono (mono)", monoAvailable),
            ("Ioskeley-Mono-Medium (mono medium)", UIFont(name: "Ioskeley-Mono-Medium", size: 12) != nil),
            ("Banga-ExtraLight (display)", displayAvailable),
            ("JunicodeVF-Regular (serif)", serifAvailable),
            ("JunicodeVF-Italic (serif italic)", UIFont(name: "JunicodeVF-Italic", size: 12) != nil),
            ("OFLGoudyStMTT (goudy)", goudyAvailable),
            ("OFLGoudyStMTT-Italic (goudy italic)", UIFont(name: "OFLGoudyStMTT-Italic", size: 12) != nil),
            ("Garamontio-Regular (garamontio)", garamontioAvailable),
            ("Garamontio-Italic (garamontio italic)", UIFont(name: "Garamontio-Italic", size: 12) != nil),
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
