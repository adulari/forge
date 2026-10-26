// GitHub-style pipe tables for the chat's Markdown renderer: a header row, a `|---|:---:|`
// delimiter row, then body rows. Models answer with them all the time ("a table of the tests");
// without this they rendered as raw `| a | b |` lines.

const DELIMITER = /^\s*\|?\s*:?-+:?\s*(\|\s*:?-+:?\s*)+\|?\s*$/;

/** Whether `line` is a table's delimiter row (`|---|---|`, `:--- | ---:`). */
export function isTableDelimiter(line: string): boolean {
  return DELIMITER.test(line);
}

/** Whether `header` followed by `next` opens a table. */
export function opensTable(header: string, next: string | undefined): boolean {
  return next !== undefined && header.includes("|") && isTableDelimiter(next);
}

/** The cells of one table row: outer pipes dropped, `\|` kept as a literal pipe, a pipe inside
 * an inline code span left alone. */
export function tableCells(line: string): string[] {
  let text = line.trim();
  if (text.startsWith("|")) text = text.slice(1);
  if (text.endsWith("|") && !text.endsWith("\\|")) text = text.slice(0, -1);
  const cells: string[] = [];
  let cell = "";
  let inCode = false;
  for (let i = 0; i < text.length; i++) {
    const ch = text[i];
    if (ch === "\\" && text[i + 1] === "|") {
      cell += "|";
      i++;
    } else if (ch === "`") {
      inCode = !inCode;
      cell += ch;
    } else if (ch === "|" && !inCode) {
      cells.push(cell.trim());
      cell = "";
    } else {
      cell += ch;
    }
  }
  cells.push(cell.trim());
  return cells;
}

/** A body row cut or padded to the header's column count, so every row lines up. */
export function fitRow(cells: string[], columns: number): string[] {
  const out = cells.slice(0, columns);
  while (out.length < columns) out.push("");
  return out;
}

export type TableAlign = "left" | "center" | "right";

/** Column alignment from the delimiter row: `:---` left, `:---:` center, `---:` right, bare
 * `---` unspecified (null, which renders left). */
export function tableAlignments(delimiter: string): (TableAlign | null)[] {
  return tableCells(delimiter).map((cell) => {
    const left = cell.startsWith(":");
    const right = cell.endsWith(":");
    if (left && right) return "center";
    if (right) return "right";
    if (left) return "left";
    return null;
  });
}

const CHAR_PX = 7.5;
const CELL_PADDING_PX = 18;
const MIN_COLUMN_PX = 72;
const MAX_COLUMN_PX = 280;

/** A basis width per column, from its longest cell: wide enough for short text on one line,
 * capped so a paragraph-long cell wraps instead of pushing the table off-screen. Markdown
 * markers (`**`, backticks, link targets) are not counted. */
export function columnWidths(rows: readonly (readonly string[])[]): number[] {
  const columns = Math.max(0, ...rows.map((row) => row.length));
  return Array.from({ length: columns }, (_, col) => {
    const longest = Math.max(
      0,
      ...rows.map((row) =>
        (row[col] ?? "")
          .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
          .replace(/[*_`]/g, "")
          .trim().length,
      ),
    );
    return Math.min(MAX_COLUMN_PX, Math.max(MIN_COLUMN_PX, Math.round(longest * CHAR_PX + CELL_PADDING_PX)));
  });
}
