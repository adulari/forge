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
