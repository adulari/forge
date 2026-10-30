import { useEffect } from "react";

import { onScopeDenied, scopeDeniedMessage } from "../lib/scopeDenied";
import { useToast } from "./ds/ToastHost";

export function ScopeDeniedToast(): null {
  const toast = useToast();
  useEffect(() => onScopeDenied((scope) => toast.show(scopeDeniedMessage(scope), { tone: "neutral" })), [toast]);
  return null;
}
