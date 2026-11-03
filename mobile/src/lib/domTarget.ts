// Event targets are not always Nodes: `window`, a document from another realm and synthetic
// events dispatched by shortcut plumbing all reach DOM handlers. WebKit throws "Argument 1 to
// Node.contains must be an instance of Node" for those (#1732), so check before asking.

function isNode(value: unknown): value is Node {
  return typeof value === "object" && value !== null && typeof (value as { nodeType?: unknown }).nodeType === "number";
}

/** `root.contains(candidate)` that is false, not a throw, when `candidate` is not a Node. */
export function containsNode(root: Node | null | undefined, candidate: unknown): boolean {
  return root != null && isNode(candidate) && root.contains(candidate);
}

/** `target.closest(selector)` for event targets that may be `window` or another non-Element. */
export function closestElement(target: unknown, selector: string): Element | null {
  const element = target as { closest?: (selector: string) => Element | null } | null;
  return typeof element?.closest === "function" ? element.closest(selector) : null;
}
