// Forge Anywhere entitlement badge — wraps ds/Badge, mapping EntitlementState to Badge's
// existing tone palette (mobile.dc.html "entitlement badge" legend, lines 1290-1299).
// The design's badges use a 10px JetBrains-mono label; ds/Badge only exposes its existing
// sans `type.meta` text and this task doesn't touch Badge.tsx, so the label renders in
// that font instead — content, casing, and tone still match the comp exactly.
import React from "react";

import { entitlementBadge } from "../../lib/anywhere/format";
import type { AnywhereAccount } from "../../lib/anywhere/types";
import { Badge } from "../ds/Badge";

export interface EntitlementBadgeProps {
  account: Pick<AnywhereAccount, "entitlement" | "trialDaysLeft" | "graceDaysLeft" | "deletesInDays">;
}

export function EntitlementBadge({ account }: EntitlementBadgeProps) {
  const { label, tone } = entitlementBadge(account);
  return <Badge label={label} tone={tone} />;
}
