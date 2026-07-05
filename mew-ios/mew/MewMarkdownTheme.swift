import SwiftUI
import UIKit
import SwiftStreamingMarkdown

// MARK: - mew markdown theme

extension MarkdownRenderConfig {
    /// mew's render config for SwiftStreamingMarkdown: MiSans for body text and
    /// headings, Ioskeley mono for code, tightened block spacing for chat
    /// bubbles. Bold/italic variants are synthesized from the MiSans variable
    /// font via symbolic traits and fall back to the regular weight if the
    /// trait can't be applied (headings then stay distinct by size). Falls back
    /// to system fonts if a face fails to load.
    static let mew: MarkdownRenderConfig = {
        MarkdownRenderConfig.default
            .withShouldAnimateText(value: true)
            .withBlockSpacing(value: 12)
            .withParagraphStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(16),
                textColor: .primary
            ))
            .withOrderedListStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(16),
                textColor: .primary
            ))
            .withBlockQuoteStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(16),
                textColor: .secondary
            ))
            .withHeadingStyle(value: MarkdownHeadingTextStyle(
                h1Font: headingFonts(26),
                h2Font: headingFonts(22),
                h3Font: headingFonts(19),
                h4Font: headingFonts(17),
                h5Font: headingFonts(16),
                h6Font: headingFonts(15),
                textColor: .primary
            ))
            .withTableStyle(value: MarkdownTableTextStyle(
                textFonts: bodyFonts(14),
                headerTextColor: .primary,
                regularTextColor: .primary,
                headerBackgroundColor: Color(.tertiarySystemBackground),
                borderColor: Color(.separator),
                actionButtonColor: .accentColor
            ))
            .withInlineStyle(value: MarkdownInlineTextStyle(
                boldTextColor: .primary,
                linkTextFont: mewUIFont("MiSansLatinVF", 16),
                linkTextColor: .accentColor,
                codeTextFont: mewUIFont("Ioskeley-Mono", 15),
                codeTextColor: .primary,
                codeBackgroundColor: Color(.tertiarySystemBackground),
                codeUnderlineColor: .clear
            ))
    }()

    /// Same as `mew` but without the text fade-in. Committed messages use this
    /// so they don't replay the stream-in animation every time they scroll
    /// off-screen and back (which remounts the view). Only the live streaming
    /// bubble should animate.
    static let mewStatic: MarkdownRenderConfig = mew.withShouldAnimateText(value: false)

    /// A MiSans font set at `size`, with italic/bold synthesized from traits.
    private static func bodyFonts(_ size: CGFloat) -> TextFonts {
        makeFonts(mewUIFont("MiSansLatinVF", size))
    }

    /// A bold MiSans font set at `size` for headings.
    private static func headingFonts(_ size: CGFloat) -> TextFonts {
        let base = mewUIFont("MiSansLatinVF", size)
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

    private static func mewUIFont(_ name: String, _ size: CGFloat) -> UIFont {
        UIFont(name: name, size: size) ?? .systemFont(ofSize: size)
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
