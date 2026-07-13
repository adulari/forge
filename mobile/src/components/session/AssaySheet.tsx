import React, { useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";

export function AssaySheet({ visible, onClose, send }: { visible: boolean; onClose: () => void; send: (input: RemoteInput) => boolean }) {
  const tokens = useTokens();
  const [scope, setScope] = useState<"repo" | "diff">("repo");
  const start = () => { if (send({ kind: "prompt", text: scope === "diff" ? "/assay --diff" : "/assay" })) onClose(); };
  return <Sheet visible={visible} onClose={onClose} accessibilityLabel="Run quality assay" snapPoints={[0.55]}><View style={{ paddingHorizontal: space.space16, paddingBottom: space.space16, gap: space.space12 }}><Text style={[type.heading, { color: tokens.ink }]}>Quality assay</Text><Text style={[type.sub, { color: tokens.ink3 }]}>Run Forge’s critic crew across correctness, safety, coverage, design, architecture, docs, and unnecessary complexity.</Text><Segmented options={[{ value: "repo", label: "Repository" }, { value: "diff", label: "Current diff" }]} value={scope} onChange={(value) => setScope(value as typeof scope)} /><Text style={[type.meta, { color: tokens.ink3 }]}>You’ll choose analysis-only or permission-gated cleanup next.</Text><Button label="Start assay" onPress={start} fullWidth /></View></Sheet>;
}
