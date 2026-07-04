import SwiftUI

/// Custom fonts bundled with the app, matching the mew web UI.
extension Font {
    // MiSans — the primary sans-serif body font
    static let mewSans = Font.custom("MiSans", size: UIFont.systemFontSize)
    static let mewSansMedium = Font.custom("MiSans", size: UIFont.systemFontSize).weight(.medium)
    static let mewSansSemibold = Font.custom("MiSans", size: UIFont.systemFontSize).weight(.semibold)
    static let mewSansBold = Font.custom("MiSans", size: UIFont.systemFontSize).weight(.bold)

    // Banga — display/headline font
    static func mewDisplay(_ size: CGFloat) -> Font {
        Font.custom("Banga-VF", size: size)
    }

    // Ioskeley Mono — code/monospaced font
    static let mewMono = Font.custom("IoskeleyMono", size: UIFont.systemFontSize)
    static let mewMonoMedium = Font.custom("IoskeleyMono", size: UIFont.systemFontSize).weight(.medium)

    // Junicode — serif body font
    static func mewSerif(_ size: CGFloat) -> Font {
        Font.custom("JunicodeVF-Roman", size: size)
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

    /// Convenience: mewSans at a specific size
    static func mewSans(_ size: CGFloat) -> Font {
        Font.custom("MiSans", size: size)
    }

    /// Convenience: IoskeleyMono at a specific size
    static func mewMono(_ size: CGFloat) -> Font {
        Font.custom("IoskeleyMono", size: size)
    }
}

/// Check if custom fonts are available at runtime.
enum MewFonts {
    static var sansAvailable: Bool {
        UIFont(name: "MiSans", size: 12) != nil
    }
    static var monoAvailable: Bool {
        UIFont(name: "IoskeleyMono", size: 12) != nil
    }
    static var displayAvailable: Bool {
        UIFont(name: "Banga-VF", size: 12) != nil
    }
    static var serifAvailable: Bool {
        UIFont(name: "JunicodeVF-Roman", size: 12) != nil
    }
    static var goudyAvailable: Bool {
        UIFont(name: "OFLGoudyStMTT", size: 12) != nil
    }
}
