import SwiftUI
import UIKit
import SwiftStreamingMarkdown

// MARK: - mew markdown theme

extension MarkdownRenderConfig {
    /// mew's render config for SwiftStreamingMarkdown: MiSans for body text,
    /// Ioskeley mono for code, tightened block spacing for chat bubbles. Bold
    /// and italic body variants are derived from the MiSans variable font via
    /// symbolic traits; they fall back to the regular weight if the trait can't
    /// be synthesized. Falls back to system fonts if a face fails to load.
    static let mew: MarkdownRenderConfig = {
        MarkdownRenderConfig.default
            .withShouldAnimateText(value: true)
            .withBlockSpacing(value: 12)
            .withParagraphStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts,
                textColor: .primary
            ))
            .withInlineStyle(value: MarkdownInlineTextStyle(
                boldTextColor: .primary,
                linkTextFont: mewFont("MiSansLatinVF", 16),
                linkTextColor: .accentColor,
                codeTextFont: mewFont("Ioskeley-Mono", 15),
                codeTextColor: .primary,
                codeBackgroundColor: Color(.tertiarySystemBackground),
                codeUnderlineColor: .clear
            ))
    }()

    private static var bodyFonts: TextFonts {
        let body = mewFont("MiSansLatinVF", 16)
        return TextFonts(
            normal: body,
            italic: body.withTraits(.traitItalic),
            bold: body.withTraits(.traitBold),
            boldItalic: body.withTraits([.traitBold, .traitItalic]),
            preferredLetterSpacing: nil,
            preferredLineHeight: nil
        )
    }

    private static func mewFont(_ name: String, _ size: CGFloat) -> UIFont {
        UIFont(name: name, size: size) ?? .systemFont(ofSize: size)
    }
}

private extension UIFont {
    /// Derive a trait variant (bold/italic) from this font, or `nil` if the
    /// trait can't be applied — `TextFonts` then falls back to the base font.
    func withTraits(_ traits: UIFontDescriptor.SymbolicTraits) -> UIFont? {
        guard let descriptor = fontDescriptor.withSymbolicTraits(traits) else { return nil }
        return UIFont(descriptor: descriptor, size: pointSize)
    }
}
