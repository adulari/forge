// TeX in chat markdown: `$$ … $$` and `\[ … \]` blocks, `$x$` and `\(x\)` inline. The dollar
// rules follow pandoc so prices survive: an opening `$` is followed by a non-space, a closing one
// is preceded by a non-space and never followed by a digit or word character ("$5 and $10" is
// money, "$x$" is math).

/** Inline alternatives for the markdown inline pattern: three capture groups. */
export const INLINE_MATH_SOURCE =
  String.raw`\$\$([^$\n]+?)\$\$|\\\((.+?)\\\)|(?<![\\$\w])\$(?![\s$])([^$\n]*?[^\s$\\])\$(?![\w$])`;

export interface MathBlock {
  tex: string;
  /** Index of the first line after the block. */
  next: number;
}

/** The display-math block starting at `lines[start]`, or null when none opens there or it never
 * closes (a streamed reply is still being written; it stays plain text until the close arrives). */
export function parseMathBlock(lines: readonly string[], start: number): MathBlock | null {
  const first = lines[start].trim();
  const dollars = first.startsWith("$$");
  const bracket = first.startsWith("\\[");
  if (!dollars && !bracket) return null;
  const open = dollars ? "$$" : "\\[";
  const close = dollars ? "$$" : "\\]";
  const rest = first.slice(open.length);
  const sameLine = rest.indexOf(close);
  if (sameLine !== -1) {
    if (rest.slice(sameLine + close.length).trim() !== "") return null;
    return { tex: rest.slice(0, sameLine).trim(), next: start + 1 };
  }
  const body = rest.trim() === "" ? [] : [rest];
  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i];
    const at = line.indexOf(close);
    if (at !== -1) {
      if (line.slice(at + close.length).trim() !== "") return null;
      const head = line.slice(0, at);
      if (head.trim() !== "") body.push(head);
      return { tex: body.join("\n").trim(), next: i + 1 };
    }
    body.push(line);
  }
  return null;
}
