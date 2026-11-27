#if !canImport(Darwin)
import Foundation
import cmark_gfm
import cmark_gfm_extensions

// The native preview asks Foundation for inline Markdown and reads its
// characters. swift-corelibs has AttributedString but lacks Apple's Markdown
// initializer and presentation-intent attributes.
struct DesktopInlineIntent: OptionSet, Hashable, Codable {
    let rawValue: Int
    static let stronglyEmphasized = Self(rawValue: 1)
    static let code = Self(rawValue: 2)
}

enum DesktopIntentKey: AttributedStringKey {
    typealias Value = DesktopInlineIntent
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

    static func plain(_ text: String) throws -> String {
        _ = extensions
        guard let parser = cmark_parser_new(0) else { throw ParseError.allocation }
        defer { cmark_parser_free(parser) }
        if let strike = cmark_find_syntax_extension("strikethrough") {
            cmark_parser_attach_syntax_extension(parser, strike)
        }
        // The original block parser supplies one inline span at a time. Keep a
        // leading '#' or list marker inside that span from starting a new block.
        let source = "x " + text
        source.withCString { cmark_parser_feed(parser, $0, source.utf8.count) }
        guard let root = cmark_parser_finish(parser) else { throw ParseError.allocation }
        defer { cmark_node_free(root) }
        guard let iterator = cmark_iter_new(root) else { throw ParseError.allocation }
        defer { cmark_iter_free(iterator) }
        var result = ""
        while true {
            let event = cmark_iter_next(iterator)
            if event == CMARK_EVENT_DONE { break }
            guard event == CMARK_EVENT_ENTER, let node = cmark_iter_get_node(iterator) else { continue }
            switch cmark_node_get_type(node) {
            case CMARK_NODE_TEXT, CMARK_NODE_CODE, CMARK_NODE_HTML_INLINE:
                if let literal = cmark_node_get_literal(node) { result += String(cString: literal) }
            case CMARK_NODE_SOFTBREAK, CMARK_NODE_LINEBREAK: result += "\n"
            default: break
            }
        }
        return String(result.dropFirst(2))
    }
}

extension AttributedString {
    init(markdown text: String, options: DesktopMarkdownOptions) throws {
        self.init(try DesktopMarkdown.plain(text))
    }
}
#endif
