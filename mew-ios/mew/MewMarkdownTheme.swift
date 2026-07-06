import SwiftUI
import UIKit
import SwiftStreamingMarkdown

// MARK: - mew markdown theme

extension MarkdownRenderConfig {
    /// mew's render config for SwiftStreamingMarkdown. The body font is driven
    /// by `fontChoice` so settings changes take effect immediately; Ioskeley
    /// mono is always used for code. Bold/italic variants are synthesized from
    /// the variable font via symbolic traits and fall back to the regular
    /// weight if the trait can't be applied (headings then stay distinct by
    /// size). Falls back to system fonts if a face fails to load.
    static func mew(fontChoice: MewFontChoice) -> MarkdownRenderConfig {
        let fontName = fontChoice.uiFontName
        return MarkdownRenderConfig.default
            .withShouldAnimateText(value: true)
            .withBlockSpacing(value: 12)
            .withParagraphStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(16, fontName: fontName),
                textColor: .primary
            ))
            .withOrderedListStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(16, fontName: fontName),
                textColor: .primary
            ))
            .withBlockQuoteStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(16, fontName: fontName),
                textColor: .secondary
            ))
            .withHeadingStyle(value: MarkdownHeadingTextStyle(
                h1Font: headingFonts(26, fontName: fontName),
                h2Font: headingFonts(22, fontName: fontName),
                h3Font: headingFonts(19, fontName: fontName),
                h4Font: headingFonts(17, fontName: fontName),
                h5Font: headingFonts(16, fontName: fontName),
                h6Font: headingFonts(15, fontName: fontName),
                textColor: .primary
            ))
            .withTableStyle(value: MarkdownTableTextStyle(
                textFonts: bodyFonts(14, fontName: fontName),
                headerTextColor: .primary,
                regularTextColor: .primary,
                headerBackgroundColor: Color(.tertiarySystemBackground),
                borderColor: Color(.separator),
                actionButtonColor: .accentColor
            ))
            .withInlineStyle(value: MarkdownInlineTextStyle(
                boldTextColor: .primary,
                linkTextFont: mewUIFont(fontName, 16),
                linkTextColor: .accentColor,
                codeTextFont: mewUIFont("Ioskeley-Mono", 15),
                codeTextColor: .primary,
                codeBackgroundColor: Color(.tertiarySystemBackground),
                codeUnderlineColor: .clear
            ))
    }

    /// Same as `mew` but without the text fade-in. Committed messages use this
    /// so they don't replay the stream-in animation every time they scroll
    /// off-screen and back (which remounts the view). Only the live streaming
    /// bubble should animate.
    static func mewStatic(fontChoice: MewFontChoice) -> MarkdownRenderConfig {
        mew(fontChoice: fontChoice).withShouldAnimateText(value: false)
    }

    /// Subdued, smaller config for reasoning traces so they read as secondary
    /// content but still render block markdown (code, lists) via the library.
    static func mewReasoning(fontChoice: MewFontChoice) -> MarkdownRenderConfig {
        mew(fontChoice: fontChoice)
            .withShouldAnimateText(value: false)
            .withBlockSpacing(value: 8)
            .withParagraphStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(13, fontName: fontChoice.uiFontName),
                textColor: .secondary
            ))
    }

    /// A font set at `size` with italic/bold synthesized from traits.
    private static func bodyFonts(_ size: CGFloat, fontName: String?) -> TextFonts {
        makeFonts(mewUIFont(fontName, size))
    }

    /// A bold font set at `size` for headings.
    private static func headingFonts(_ size: CGFloat, fontName: String?) -> TextFonts {
        let base = mewUIFont(fontName, size)
        return makeFonts(base.mewWithTraits(.traitBold) ?? base)
    }

    private static func makeFonts(_ base: UIFont) -> TextFonts {
        TextFonts(
            normal: base,
            italic: base.mewWithTraits(.traitItalic),
            bold: base.mewWithTraits(.traitBold),
            boldItalic: base.mewWithTraits([.traitBold, .traitItalic]),
            preferredLetterSpacing: nil,
            preferredLineHeight: nil
        )
    }

    private static func mewUIFont(_ name: String?, _ size: CGFloat) -> UIFont {
        if let name {
            UIFont(name: name, size: size) ?? .systemFont(ofSize: size)
        } else {
            .systemFont(ofSize: size)
        }
    }
}

private extension UIFont {
    /// Derive a trait variant (bold/italic) from this font, or `nil` if the
    /// trait can't be applied — `TextFonts` then falls back to the base font.
    func mewWithTraits(_ traits: UIFontDescriptor.SymbolicTraits) -> UIFont? {
        guard let descriptor = fontDescriptor.withSymbolicTraits(traits) else { return nil }
        return UIFont(descriptor: descriptor, size: pointSize)
    }
}
