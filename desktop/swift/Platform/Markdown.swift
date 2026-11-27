#if !canImport(Darwin)
import Foundation
import cmark_gfm
import cmark_gfm_extensions

// The native preview asks Foundation for inline Markdown and reads its
// characters. swift-corelibs has AttributedString but lacks Apple's Markdown
// initializer and presentation-intent attributes.
struct InlinePresentationIntent: OptionSet, Hashable, Codable {
    let rawValue: Int
    static let stronglyEmphasized = Self(rawValue: 1)
    static let code = Self(rawValue: 2)
    static let emphasized = Self(rawValue: 4)
    static let strikethrough = Self(rawValue: 8)
}

enum DesktopIntentKey: AttributedStringKey {
    typealias Value = InlinePresentationIntent
    static let name = "CodeIsland.inlinePresentationIntent"
}
struct DesktopInlineAttributes: AttributeScope { let inlinePresentationIntent: DesktopIntentKey }
extension AttributeDynamicLookup {
    subscript<T: AttributedStringKey>(dynamicMember key: KeyPath<DesktopInlineAttributes, T>) -> T {
        self[T.self]
    }
}

struct DesktopMarkdownOptions {
    enum Syntax { case inlineOnlyPreservingWhitespace }
    let interpretedSyntax: Syntax
}

private enum DesktopMarkdown {
    static let extensions: Void = cmark_gfm_core_extensions_ensure_registered()
    enum ParseError: Error { case allocation }

    static func attributed(_ text: String) throws -> AttributedString {
        _ = extensions
        guard let parser = cmark_parser_new(0) else { throw ParseError.allocation }
        defer { cmark_parser_free(parser) }
        for name in ["strikethrough", "autolink"] {
            if let ext = cmark_find_syntax_extension(name) { cmark_parser_attach_syntax_extension(parser, ext) }
        }
        // The original block parser supplies one inline span at a time. Keep a
        // leading '#' or list marker inside that span from starting a new block.
        let source = "x " + text
        source.withCString { cmark_parser_feed(parser, $0, source.utf8.count) }
        guard let root = cmark_parser_finish(parser) else { throw ParseError.allocation }
        defer { cmark_node_free(root) }
        guard let iterator = cmark_iter_new(root) else { throw ParseError.allocation }
        defer { cmark_iter_free(iterator) }
        var result = AttributedString()
        while true {
            let event = cmark_iter_next(iterator)
            if event == CMARK_EVENT_DONE { break }
            guard event == CMARK_EVENT_ENTER, let node = cmark_iter_get_node(iterator) else { continue }
            let literal: String
            switch cmark_node_get_type(node) {
            case CMARK_NODE_TEXT, CMARK_NODE_CODE, CMARK_NODE_HTML_INLINE:
                guard let bytes = cmark_node_get_literal(node) else { continue }
                literal = String(cString: bytes)
            case CMARK_NODE_SOFTBREAK, CMARK_NODE_LINEBREAK: literal = "\n"
            default: continue
            }
            var span = AttributedString(literal)
            var intent: InlinePresentationIntent = []
            var current = Optional(node)
            while let ancestor = current {
                switch cmark_node_get_type(ancestor) {
                case CMARK_NODE_EMPH: intent.insert(.emphasized)
                case CMARK_NODE_STRONG: intent.insert(.stronglyEmphasized)
                case CMARK_NODE_CODE: intent.insert(.code)
                case CMARK_NODE_LINK:
                    if let url = cmark_node_get_url(ancestor) { span.link = URL(string: String(cString: url)) }
                default:
                    if String(cString: cmark_node_get_type_string(ancestor)) == "strikethrough" { intent.insert(.strikethrough) }
                }
                current = cmark_node_parent(ancestor)
            }
            if !intent.isEmpty { span.inlinePresentationIntent = intent }
            result += span
        }
        return AttributedString(result[result.characters.dropFirst(2).startIndex...])
    }
}

extension AttributedString {
    init(markdown text: String, options: DesktopMarkdownOptions) throws {
        self = try DesktopMarkdown.attributed(text)
    }
}
#endif
