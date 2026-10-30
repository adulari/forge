import React, { useState } from "react";

import { deadPin } from "../../lib/modelCatalog";
import { useModels } from "../../lib/queries";
import { Banner } from "../ds/Banner";

export interface DeadPinBannerProps {
  model: string | null | undefined;
  pinned: boolean | undefined;
  onReplace: (model: string) => void;
}

export function DeadPinBanner({ model, pinned, onReplace }: DeadPinBannerProps) {
  const query = useModels();
  const [dismissed, setDismissed] = useState<string | null>(null);
  if (!pinned || !model || dismissed === model) return null;
  const dead = deadPin(query.data, model);
  if (!dead) return null;
  const { replacement } = dead;
  return (
    <Banner
      tone="warn"
      message={`Pinned model ${model} is unavailable: ${dead.reason}.${replacement ? ` Nearest replacement: ${replacement.model.name}.` : ""}`}
      actionLabel={replacement ? `Switch to ${replacement.model.name}` : undefined}
      onAction={replacement ? () => onReplace(replacement.model.id) : undefined}
      onDismiss={() => setDismissed(model)}
    />
  );
}
