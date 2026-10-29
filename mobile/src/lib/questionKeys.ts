/** The option a bare 1-9 keypress picks, or null when the key is not a digit or the question has
 * fewer rows than that. Modified presses and text-field typing never reach here. */
export function optionIndexForKey(key: string, optionCount: number): number | null {
  if (!/^[1-9]$/.test(key)) return null;
  const index = Number(key) - 1;
  return index < optionCount ? index : null;
}

/** Selection after a press on a row. A single-choice row that is already selected clears; any
 * other press selects it, and a multi-choice row toggles. */
export function nextSelection(selected: readonly string[], label: string, multi: boolean): string[] {
  const has = selected.includes(label);
  if (multi) return has ? selected.filter((l) => l !== label) : [...selected, label];
  return has ? [] : [label];
}
