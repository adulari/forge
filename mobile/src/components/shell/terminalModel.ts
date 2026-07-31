export const MAX_TERMINALS_PER_SESSION = 8;

function numericSuffix(value: string): number | null {
  const match = /^term-(\d+)$/.exec(value);
  if (!match) return null;
  const number = Number(match[1]);
  return Number.isSafeInteger(number) && number > 0 ? number : null;
}

export function compareTerminalIds(left: string, right: string): number {
  const leftNumber = numericSuffix(left);
  const rightNumber = numericSuffix(right);
  if (leftNumber != null && rightNumber != null) return leftNumber - rightNumber;
  return left.localeCompare(right);
}

export function nextTerminalId(ids: readonly string[]): string | null {
  if (new Set(ids).size >= MAX_TERMINALS_PER_SESSION) return null;
  const used = new Set(ids);
  for (let index = 1; index <= MAX_TERMINALS_PER_SESSION; index += 1) {
    const candidate = `term-${index}`;
    if (!used.has(candidate)) return candidate;
  }
  return null;
}

export function terminalTitle(id: string): string {
  const suffix = numericSuffix(id);
  return suffix == null ? id : `Terminal ${suffix}`;
}
