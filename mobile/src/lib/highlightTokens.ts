export type TokenKind = "plain" | "keyword" | "string" | "comment" | "number";
export interface HighlightToken {
  kind: TokenKind;
  text: string;
}

const HL_ALIAS: Record<string, string> = {
  ts: "js", tsx: "js", jsx: "js", javascript: "js", typescript: "js", py: "python", python3: "python", rs: "rust", sh: "bash", shell: "bash", zsh: "bash", console: "bash", golang: "go", jsonc: "json",
};
const HL_KW: Record<string, string> = {
  rust: "as async await break const continue crate dyn else enum extern false fn for if impl in let loop match mod move mut pub ref return self Self static struct super trait true type unsafe use where while",
  js: "async await break case catch class const default delete do else export extends false finally for from function if import in instanceof let new null of return static switch this throw true try typeof undefined var void while yield",
  python: "and as assert async await break class const continue def del elif else except False finally for from global if import in is lambda None nonlocal not or pass raise return self True try while with yield",
  go: "break case chan const continue default defer else fallthrough false for func go goto if import interface map nil package range return select struct switch true type var",
  bash: "case do done echo elif else esac exit export fi for function if in local return set shift then until while",
  json: "false null true",
};

export function highlightTokens(code: string, lang: string): HighlightToken[] {
  const tokens: HighlightToken[] = [];
  const resolved = HL_ALIAS[lang] ?? lang;
  const kw = HL_KW[resolved];
  if (!kw) return [{ kind: "plain", text: code }];
  const kws = new Set(kw.split(" "));
  const lineComment = resolved === "python" || resolved === "bash" ? "#" : resolved === "json" ? null : "//";
  const blockComment = resolved === "rust" || resolved === "js" || resolved === "go" ? (["/*", "*/"] as const) : null;
  let i = 0;
  let plain = "";
  const flush = () => { if (plain) { tokens.push({ kind: "plain", text: plain }); plain = ""; } };
  const push = (kind: TokenKind, text: string) => { flush(); tokens.push({ kind, text }); };
  while (i < code.length) {
    const c = code[i];
    if (lineComment && code.startsWith(lineComment, i)) {
      let j = code.indexOf("\n", i); if (j < 0) j = code.length; push("comment", code.slice(i, j)); i = j; continue;
    }
    if (blockComment && code.startsWith(blockComment[0], i)) {
      let j = code.indexOf(blockComment[1], i + 2); j = j < 0 ? code.length : j + 2; push("comment", code.slice(i, j)); i = j; continue;
    }
    if (c === '"' || c === "'" || c === "`") {
      let j = i + 1; while (j < code.length && code[j] !== c && code[j] !== "\n") { if (code[j] === "\\") j++; j++; }
      j = Math.min(j + 1, code.length); push("string", code.slice(i, j)); i = j; continue;
    }
    if (/[0-9]/.test(c) && !/[A-Za-z0-9_]/.test(code[i - 1] ?? "")) {
      let j = i; while (j < code.length && /[0-9a-fA-FxXoObB._]/.test(code[j])) j++; push("number", code.slice(i, j)); i = j; continue;
    }
    if (/[A-Za-z_]/.test(c)) {
      let j = i; while (j < code.length && /[A-Za-z0-9_]/.test(code[j])) j++; const word = code.slice(i, j);
      if (kws.has(word)) push("keyword", word); else plain += word; i = j; continue;
    }
    plain += c; i++;
  }
  flush();
  return tokens;
}
