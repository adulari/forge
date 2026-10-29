// Chat markdown, parsed without a library: chat content is controlled (assistant and tool
// output), not arbitrary user markdown. Blocks are headings, paragraphs, lists, code, pipe tables,
// display math and images; inline are code, bold, italic, links, images, math and repo paths.
import { INLINE_MATH_SOURCE, parseMathBlock } from "./markdownMath";
import { columnWidths, fitRow, opensTable, tableAlignments, tableCells, type TableAlign } from "./markdownTable";
import { pathRefOfCode, splitMentions, splitPathRefs, type PathRef } from "./pathRefs";

// ---------------------------------------------------------------------------
// Block parsing
// ---------------------------------------------------------------------------

export type Block =
  | { kind: "heading"; level: number; text: string; inline: InlineNode[] }
  | { kind: "paragraph"; text: string; inline: InlineNode[] }
  | { kind: "list"; ordered: boolean; start: number; items: string[]; inlineItems: InlineNode[][] }
  | { kind: "code"; language: string; code: string }
  | { kind: "math"; tex: string }
  | { kind: "image"; alt: string; src: string }
  | { kind: "table"; header: InlineNode[][]; rows: InlineNode[][][]; aligns: (TableAlign | null)[]; widths: number[] };

function fitAligns(aligns: (TableAlign | null)[], columns: number): (TableAlign | null)[] {
  const out = aligns.slice(0, columns);
  while (out.length < columns) out.push(null);
  return out;
}

export function parseBlocks(content: string, mentions: boolean): Block[] {
  const parseInline = (text: string) => parseInlineNodes(text, mentions);
  const lines = content.replace(/\r\n/g, "\n").split("\n");
  const blocks: Block[] = [];
  let paragraphLines: string[] = [];
  let listItems: string[] = [];
  let listOrdered = false;
  // A loose list ("1. a", blank line, "2. b") arrives as one list per item; numbering from the
  // source's own first number keeps it 1, 2 instead of restarting at 1.
  let listStart = 1;

  const flushParagraph = () => {
    if (paragraphLines.length) {
      const images = paragraphLines.map((line) => LONE_IMAGE_RE.exec(line.trim()));
      if (images.every((match) => match !== null)) {
        for (const match of images) blocks.push({ kind: "image", alt: match![1], src: match![2] });
      } else {
        const text = paragraphLines.join("\n");
        blocks.push({ kind: "paragraph", text, inline: parseInline(text) });
      }
      paragraphLines = [];
    }
  };
  const flushList = () => {
    if (listItems.length) {
      blocks.push({ kind: "list", ordered: listOrdered, start: listStart, items: listItems, inlineItems: listItems.map((item) => parseInline(item)) });
      listItems = [];
    }
  };

  let i = 0;
  while (i < lines.length) {
    const line = lines[i];

    const fence = /^```\s*([\w-]*)\s*$/.exec(line);
    if (fence) {
      flushParagraph();
      flushList();
      const language = fence[1] ?? "";
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !/^```\s*$/.test(lines[i])) {
        codeLines.push(lines[i]);
        i++;
      }
      i++; // skip closing fence
      blocks.push({ kind: "code", language, code: codeLines.join("\n") });
      continue;
    }

    const math = parseMathBlock(lines, i);
    if (math) {
      flushParagraph();
      flushList();
      blocks.push({ kind: "math", tex: math.tex });
      i = math.next;
      continue;
    }

    const heading = /^(#{1,6})\s+(.*)$/.exec(line);
    if (heading) {
      flushParagraph();
      flushList();
      const text = heading[2].trim();
      blocks.push({ kind: "heading", level: heading[1].length, text, inline: parseInline(text) });
      i++;
      continue;
    }

    if (opensTable(line, lines[i + 1])) {
      flushParagraph();
      flushList();
      const header = tableCells(line);
      const aligns = fitAligns(tableAlignments(lines[i + 1]), header.length);
      const rawRows: string[][] = [];
      i += 2;
      while (i < lines.length && lines[i].includes("|") && lines[i].trim() !== "") {
        rawRows.push(fitRow(tableCells(lines[i]), header.length));
        i++;
      }
      blocks.push({
        kind: "table",
        header: header.map((cell) => parseInline(cell)),
        rows: rawRows.map((row) => row.map((cell) => parseInline(cell))),
        aligns,
        widths: columnWidths([header, ...rawRows]),
      });
      continue;
    }

    const unordered = /^\s*[-*]\s+(.*)$/.exec(line);
    const ordered = /^\s*(\d+)\.\s+(.*)$/.exec(line);
    if (unordered || ordered) {
      flushParagraph();
      const nowOrdered = Boolean(ordered);
      if (listItems.length && nowOrdered !== listOrdered) flushList();
      if (!listItems.length) listStart = ordered ? Number(ordered[1]) : 1;
      listOrdered = nowOrdered;
      listItems.push(ordered ? ordered[2] : unordered![1]);
      i++;
      continue;
    }

    if (line.trim() === "") {
      flushParagraph();
      flushList();
      i++;
      continue;
    }

    flushList();
    paragraphLines.push(line);
    i++;
  }
  flushParagraph();
  flushList();
  return blocks;
}

// ---------------------------------------------------------------------------
// Inline parsing: code, bold, italic, links, images, math, repo paths.
// ---------------------------------------------------------------------------

export type InlineNode =
  | { type: "text"; text: string }
  | { type: "bold"; text: string }
  | { type: "italic"; text: string }
  | { type: "code"; text: string }
  | { type: "link"; text: string; href: string }
  | { type: "image"; alt: string; src: string }
  | { type: "math"; tex: string }
  | { type: "path"; text: string; ref: PathRef };

const IMAGE_SOURCE = String.raw`!\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)`;
const LONE_IMAGE_RE = new RegExp(`^${IMAGE_SOURCE}$`);
// Groups: 1 code, 2 bold, 3 `*italic*`, 4 `_italic_` (never inside a word, so `my_file_name.ts`
// stays a path), 5-6 link, 7-8 image, 9-11 math.
const INLINE_RE = new RegExp(
  [
    "`([^`]+)`",
    String.raw`\*\*([^*]+)\*\*`,
    String.raw`\*([^*]+)\*`,
    String.raw`(?<![\w])_([^_]+)_(?![\w])`,
    String.raw`\[([^\]]+)\]\(([^)]+)\)`,
    IMAGE_SOURCE,
    INLINE_MATH_SOURCE,
  ].join("|"),
  "g",
);

function pushText(nodes: InlineNode[], text: string, mentions: boolean) {
  const pieces = mentions ? splitMentions(text) : [{ type: "text" as const, text }];
  for (const piece of pieces) {
    if (piece.type === "path") {
      nodes.push({ type: "path", text: piece.text, ref: piece.ref });
      continue;
    }
    for (const part of splitPathRefs(piece.text)) {
      nodes.push(part.type === "path" ? { type: "path", text: part.text, ref: part.ref } : { type: "text", text: part.text });
    }
  }
}

export function parseInlineNodes(text: string, mentions: boolean): InlineNode[] {
  const nodes: InlineNode[] = [];
  let lastIndex = 0;
  INLINE_RE.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = INLINE_RE.exec(text))) {
    if (match.index > lastIndex) pushText(nodes, text.slice(lastIndex, match.index), mentions);
    if (match[1] !== undefined) {
      const ref = pathRefOfCode(match[1]);
      nodes.push(ref ? { type: "path", text: match[1].trim(), ref } : { type: "code", text: match[1] });
    } else if (match[2] !== undefined) nodes.push({ type: "bold", text: match[2] });
    else if (match[3] !== undefined) nodes.push({ type: "italic", text: match[3] });
    else if (match[4] !== undefined) nodes.push({ type: "italic", text: match[4] });
    else if (match[5] !== undefined && match[6] !== undefined) nodes.push({ type: "link", text: match[5], href: match[6] });
    else if (match[7] !== undefined && match[8] !== undefined) nodes.push({ type: "image", alt: match[7], src: match[8] });
    else {
      const tex = match[9] ?? match[10] ?? match[11];
      if (tex !== undefined) nodes.push({ type: "math", tex });
    }
    lastIndex = INLINE_RE.lastIndex;
  }
  if (lastIndex < text.length) pushText(nodes, text.slice(lastIndex), mentions);
  return nodes;
}
