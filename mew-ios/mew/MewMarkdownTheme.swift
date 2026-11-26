import SwiftUI
import UIKit
import SwiftStreamingMarkdown

// MARK: - File reference directives

/// Converts `:file[label]{path="..."}` into the markdown citation links that
/// SwiftStreamingMarkdown renders as inline pills. The marker is custom so
/// ordinary external citations keep their own format.
enum FileDirective {
    static let marker = "mew-file"

    static let coder = CitationCoder(
        citationMarker: marker,
        citationMarkerQueryParam: "citationMarker",
        citationTextQueryParam: "citationTitle",
        citationA11yTextQueryParam: "citationA11yValue"
    )

    private static let regex = try! NSRegularExpression(
        pattern: #":file\[([^\]\n]+)\]\{path=(?:"([^"\n]+)"|“([^”\n]+)”|([^\s}\n]+))\}"#
    )

    static func markdown(_ text: String) -> String {
        let ns = text as NSString
        let matches = regex.matches(in: text, range: NSRange(location: 0, length: ns.length))
        var result = ""
        var cursor = 0
        for match in matches {
            result += ns.substring(with: NSRange(location: cursor, length: match.range.location - cursor))
            let label = ns.substring(with: match.range(at: 1))
            let pathGroup = [2, 3, 4].first { match.range(at: $0).location != NSNotFound } ?? 4
            result += citationLink(label: label, path: ns.substring(with: match.range(at: pathGroup)))
            cursor = match.range.location + match.range.length
        }
        result += ns.substring(from: cursor)
        return result
    }

    static func path(from url: URL) -> String? {
        guard url.scheme == marker,
              let items = URLComponents(url: url, resolvingAgainstBaseURL: true)?.queryItems,
              items.contains(where: { $0.name == "citationMarker" && $0.value == marker })
        else { return nil }
        let path = items.first { $0.name == "path" }?.value ?? ""
        return path.isEmpty ? nil : path
    }

    private static func citationLink(label: String, path: String) -> String {
        "[\(marker)](\(linkURL(label: label, path: path).absoluteString))"
    }

    static func linkURL(label: String, path: String) -> URL {
        var components = URLComponents()
        components.scheme = marker
        components.host = "open"
        components.queryItems = [
            URLQueryItem(name: "citationMarker", value: marker),
            URLQueryItem(name: "citationTitle", value: label),
            URLQueryItem(name: "citationA11yValue", value: "Open \(label)"),
            URLQueryItem(name: "path", value: path),
        ]
        return components.url!
    }
}

// MARK: - mew markdown theme

extension MarkdownRenderConfig {
    /// mew's render config for SwiftStreamingMarkdown. The body font is driven
    /// by `fontChoice` so settings changes take effect immediately; Ioskeley
    /// mono is always used for code. Bold/italic variants are synthesized from
    /// the variable font via symbolic traits and fall back to the regular
    /// weight if the trait can't be applied (headings then stay distinct by
    /// size). Falls back to system fonts if a face fails to load.
    static func mew(fontChoice: MewFontChoice) -> MarkdownRenderConfig {
        return MarkdownRenderConfig.default
            .withShouldAnimateText(value: true)
            .withBlockSpacing(value: 12)
            .withParagraphStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(UIFont.systemFontSize, fontChoice: fontChoice),
                textColor: .primary
            ))
            .withOrderedListStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(UIFont.systemFontSize, fontChoice: fontChoice),
                textColor: .primary
            ))
            .withBlockQuoteStyle(value: MarkdownTextStyle(
                textFonts: bodyFonts(UIFont.systemFontSize, fontChoice: fontChoice),
                textColor: .secondary
            ))
            .withHeadingStyle(value: MarkdownHeadingTextStyle(
                h1Font: headingFonts(26, fontChoice: fontChoice),
                h2Font: headingFonts(22, fontChoice: fontChoice),
                h3Font: headingFonts(19, fontChoice: fontChoice),
                h4Font: headingFonts(17, fontChoice: fontChoice),
                h5Font: headingFonts(16, fontChoice: fontChoice),
                h6Font: headingFonts(15, fontChoice: fontChoice),
                textColor: .primary
            ))
            .withTableStyle(value: MarkdownTableTextStyle(
                textFonts: bodyFonts(14, fontChoice: fontChoice),
                headerTextColor: .primary,
                regularTextColor: .primary,
                headerBackgroundColor: Color(.tertiarySystemBackground),
                borderColor: Color(.separator),
                actionButtonColor: .accentColor
            ))
            .withInlineStyle(value: MarkdownInlineTextStyle(
                boldTextColor: .primary,
                linkTextFont: fontChoice.uiFont(UIFont.systemFontSize),
                linkTextColor: .accentColor,
                codeTextFont: UIFont(name: "Ioskeley-Mono", size: 15) ?? .monospacedSystemFont(ofSize: 15, weight: .regular),
                codeTextColor: .primary,
                codeBackgroundColor: Color(.tertiarySystemBackground),
                codeUnderlineColor: .clear
            ))
            .withFileCitations(fontChoice: fontChoice)
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
                textFonts: bodyFonts(13, fontChoice: fontChoice),
                textColor: .secondary
            ))
    }

    /// Set the citation format used for inline `:file` reference pills.
    private func withFileCitations(fontChoice: MewFontChoice) -> MarkdownRenderConfig {
        MarkdownRenderConfig(
            shouldAnimateText: shouldAnimateText,
            blockQuoteStyle: blockQuoteStyle,
            headingStyle: headingStyle,
            orderedListStyle: orderedListStyle,
            paragraphStyle: paragraphStyle,
            tableStyle: tableStyle,
            inlineStyle: inlineStyle,
            textContextMenu: textContextMenu,
            citationConfig: MarkdownRenderConfig.CitationConfig(
                isEnabled: true,
                coder: FileDirective.coder,
                font: fontChoice.uiFont(13),
                textColor: .primary,
                backgroundColor: Color(.tertiarySystemFill)
            ),
            blockSpacing: blockSpacing
        )
    }

    /// A font set at `size` with italic/bold synthesized from traits.
    private static func bodyFonts(_ size: CGFloat, fontChoice: MewFontChoice) -> TextFonts {
        makeFonts(fontChoice.uiFont(size))
    }

    /// A bold font set at `size` for headings.
    private static func headingFonts(_ size: CGFloat, fontChoice: MewFontChoice) -> TextFonts {
        let base = fontChoice.uiFont(size)
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
}

private extension UIFont {
    /// Derive a trait variant (bold/italic) from this font, or `nil` if the
    /// trait can't be applied — `TextFonts` then falls back to the base font.
    func mewWithTraits(_ traits: UIFontDescriptor.SymbolicTraits) -> UIFont? {
        guard let descriptor = fontDescriptor.withSymbolicTraits(traits) else { return nil }
        return UIFont(descriptor: descriptor, size: pointSize)
    }
}
