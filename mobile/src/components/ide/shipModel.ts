// The Ship row as plain data: which steps "Ship" will run for the current working tree, and what to
// call it. Staging is a step only when nothing is staged yet; a message is generated only when the
// box is empty; push only when the repository has a remote.
export type ShipStep = "stage" | "message" | "commit" | "push";

export interface ShipInput {
  staged: number;
  unstaged: number;
  hasMessage: boolean;
  canPush: boolean;
}

export function shipSteps({ staged, unstaged, hasMessage, canPush }: ShipInput): ShipStep[] {
  if (staged + unstaged === 0) return [];
  const steps: ShipStep[] = [];
  if (staged === 0) steps.push("stage");
  if (!hasMessage) steps.push("message");
  steps.push("commit");
  if (canPush) steps.push("push");
  return steps;
}

const WORD: Record<ShipStep, string> = { stage: "stage", message: "write a message", commit: "commit", push: "push" };

export function shipHint(steps: readonly ShipStep[]): string {
  if (steps.length === 0) return "Nothing to ship. The working tree is clean.";
  const words = steps.map((step) => WORD[step]);
  const last = words.pop() as string;
  return `Will ${words.length > 0 ? `${words.join(", ")} and ${last}` : last}.`;
}

export function shipLabel(steps: readonly ShipStep[]): string {
  return steps.includes("push") ? "Ship it" : "Commit";
}

export type PostShipAction = { kind: "none" } | { kind: "open-pr" } | { kind: "view-pr"; number: number };

// After a push: offer the existing pull request when the branch has one, otherwise offer to open one.
export function postShipAction(pushed: boolean, pr: { number: number } | null | undefined): PostShipAction {
  if (!pushed) return { kind: "none" };
  return pr ? { kind: "view-pr", number: pr.number } : { kind: "open-pr" };
}

export function failedChecksWarning(names: readonly string[]): string | null {
  if (names.length === 0) return null;
  return `Last check failed: ${names.slice(0, 2).join(", ")}${names.length > 2 ? ` +${names.length - 2}` : ""}.`;
}
