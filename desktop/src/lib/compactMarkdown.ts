/// <reference lib="es2022.intl" />

import type { Nodes } from "mdast";
import remarkGfm from "remark-gfm";
import remarkParse from "remark-parse";
import { unified } from "unified";

const markdown = unified().use(remarkParse).use(remarkGfm).freeze();
const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });
const SOURCE_LIMIT = 8 * 1024;
const PREVIEW_LIMIT = 160;

function joinBlocks(blocks: string[]): string {
  return blocks.map((block) => block.trim()).filter(Boolean).reduce((preview, block) => {
    if (!preview) return block;
    const separator = /[:：]$/.test(preview) ? " " : " · ";
    return `${preview}${separator}${block}`;
  }, "");
}

function plain(node: Nodes): string {
  switch (node.type) {
    case "text":
    case "inlineCode":
      return node.value;
    case "image":
    case "imageReference":
      return node.alt ?? "";
    case "break":
      return " ";
    case "heading":
    case "paragraph":
    case "tableCell":
    case "emphasis":
    case "strong":
    case "delete":
    case "link":
    case "linkReference":
      return node.children.map(plain).join("");
    case "root":
    case "blockquote":
      return joinBlocks(node.children.map(plain));
    case "list":
      return joinBlocks(node.children.map((item, index) => {
        const text = plain(item);
        return text && node.ordered ? `${(node.start ?? 1) + index}. ${text}` : text;
      }));
    case "listItem": {
      const text = joinBlocks(node.children.map(plain));
      if (node.checked == null) return text;
      return `${node.checked ? "☑" : "☐"} ${text}`;
    }
    case "table":
      return joinBlocks(node.children.map(plain));
    case "tableRow":
      return node.children.map(plain).map((cell) => cell.trim()).filter(Boolean).join(", ");
    case "code": {
      const lines = node.value.split("\n").map((line) => line.trim()).filter(Boolean);
      return lines.length > 1 ? `${lines[0]} …` : lines[0] ?? "";
    }
    default:
      return "";
  }
}

/** A bounded one-line preview of the current assistant stream. */
export function compactMarkdownPreview(text: string): string {
  const preview = plain(markdown.parse(text.slice(0, SOURCE_LIMIT))).replace(/\s+/g, " ").trim();
  let count = 0;
  let lastIndex = 0;
  for (const { index } of graphemes.segment(preview)) {
    if (count === PREVIEW_LIMIT) return `${preview.slice(0, lastIndex)}…`;
    lastIndex = index;
    count += 1;
  }
  return preview;
}
