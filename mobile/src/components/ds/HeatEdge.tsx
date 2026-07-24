// Machined retires the thermal identity (forge wash / heat edges / dot glows) —
// this component now renders nothing; kept only so existing imports/props compile.
// Wave 2 removes the call sites (Card's `heatEdge` prop, Composer, floor tiles).
export interface HeatEdgeProps {
  state?: "busy" | "waiting" | false;
  /** @deprecated legacy boolean API — kept so existing call sites still compile. */
  active?: boolean;
}

export function HeatEdge(_props: HeatEdgeProps) {
  return null;
}
