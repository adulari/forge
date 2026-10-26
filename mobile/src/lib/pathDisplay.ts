// How paths and commands read in a one-line row. The app does not know the host's home folder, so
// the usual shapes (`/home/<user>`, `/Users/<user>`, `/root`) fold to `~`, as the TUI does for the
// host's own home (crates/forge-tui/src/app/card_text.rs `shorten_in`).

const HOME = /(^|[\s"'`=:(,;|&<>])(?:\/home\/[^/\s"'`]+|\/Users\/[^/\s"'`]+|\/root)(?=\/|$|[\s"'`);|&<>])/g;

/** `text` with the home folder written as `~` wherever a path appears in it. */
export function tildify(text: string): string {
  return text.replace(HOME, "$1~");
}

/** A path shortened to at most `max` characters: the home as `~`, and when still too long the
 * middle dropped, keeping the first segment and the filename (`~/…/logs/run.log`). */
export function shortenPath(path: string, max = 48): string {
  const home = tildify(path.trim());
  if (home.length <= max) return home;
  const parts = home.split("/");
  const file = parts[parts.length - 1] || parts[parts.length - 2] || home;
  if (parts.length <= 2) return middleTruncate(home, max);
  const lead = parts[0] === "" ? "/" + (parts[1] ?? "") : parts[0];
  let tailParts = [file];
  const build = (segments: string[]) => `${lead}/…/${segments.join("/")}`;
  for (let i = parts.length - 2; i > 1; i--) {
    const next = [parts[i], ...tailParts];
    if (build(next).length > max) break;
    tailParts = next;
  }
  const out = build(tailParts);
  return out.length <= max ? out : middleTruncate(home, max);
}

function middleTruncate(text: string, max: number): string {
  if (text.length <= max) return text;
  const head = Math.ceil((max - 1) / 2);
  const tail = Math.floor((max - 1) / 2);
  return `${text.slice(0, head)}…${text.slice(text.length - tail)}`;
}

/** A shell command as one readable line: no leading `exec`, whitespace collapsed, home as `~`. */
export function friendlyCommand(command: string): string {
  const flat = command.replace(/\s+/g, " ").trim();
  return tildify(flat.replace(/^exec\s+/, ""));
}

/** The program a command runs, by its basename (`exec /home/u/bin/agentlive-exec x` ->
 * `agentlive-exec`), skipping `env`, `sudo`-style wrappers and `VAR=value` prefixes. */
export function commandProgram(command: string): string {
  const words = friendlyCommand(command).split(" ");
  const skip = new Set(["env", "nohup", "time", "command"]);
  for (const word of words) {
    if (!word || skip.has(word) || /^[A-Za-z_][A-Za-z0-9_]*=/.test(word)) continue;
    const bare = word.replace(/^['"]|['"]$/g, "");
    const base = bare.slice(bare.lastIndexOf("/") + 1);
    return base || bare;
  }
  return "";
}
