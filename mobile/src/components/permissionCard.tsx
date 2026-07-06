// Permission approval card (BUILD_PLAN §1.3 / §6 Chat action cards, Batch 3).
//
// Renders `snapshot.permission_prompt` as an urgent Allow/Deny card. Mirrors the web control
// page's permission block (crates/forge-cli/src/remote_assets/app.js `renderActions`
// lines 854-857 + styles.css lines 55-56/71-72): an accent-bordered `.prompt` card with a
// "⚠" prefix and bold accent text, followed by Allow (`--ok`) / Deny (`--no`) buttons in that
// order. `permission_prompt` is a pre-formatted plain string from the server (see
// crates/forge-cli/src/remote.rs field + push.rs test fixtures, e.g.
// "allow write_file (Write) [y/n]" / "allow shell (Shell) [y/n]") — there is no structured
// tool/path breakdown to parse. When the text looks command-shaped (contains a path, pipe,
// backtick, or newline) it additionally renders in a mono `codeBg` block for readability;
// otherwise it's shown as bold accent text only, matching the web card exactly.
//
// prompt_seq discipline (UI_RULES.md #16): buttons send the `seq` this card was rendered
// from and disable immediately on tap. They only re-enable if a genuinely NEW prompt (a
// different seq) replaces this one — a stale tap can never resolve a newer prompt.
import React, { useEffect, useState } from "react";
import { Text, View } from "react-native";

import type { RemoteInput } from "../lib/ws";
import { ConfirmButton } from "./ui";

export interface PermissionCardProps {
  permissionPrompt: string;
  seq: number;
  send: (input: RemoteInput) => void;
}

const COMMAND_LIKE_RE = /[\n`|]|(?:--\w)|(?:\S+\/\S+)/;

export function PermissionCard({ permissionPrompt, seq, send }: PermissionCardProps) {
  // Tracks the seq we've already answered so this card disables right after a tap, and
  // re-enables only once the server hands us a genuinely different prompt_seq.
  const [answeredSeq, setAnsweredSeq] = useState<number | null>(null);

  useEffect(() => {
    setAnsweredSeq((prev) => (prev !== null && prev !== seq ? null : prev));
  }, [seq]);

  const disabled = answeredSeq === seq;
  const commandLike = COMMAND_LIKE_RE.test(permissionPrompt);

  const decide = (yes: boolean) => {
    if (disabled) return;
    setAnsweredSeq(seq);
    send({ kind: "allow", yes, seq });
  };

  return (
    <View className="bg-panel border border-accent rounded-lg px-10 py-10 gap-8">
      <View className="flex-row items-start gap-6">
        <Text className="text-accent text-[15px] font-bold">⚠</Text>
        {commandLike ? (
          <View className="flex-1 gap-4">
            <Text className="text-accent text-[13px] font-bold">Permission needed</Text>
            <View className="bg-codeBg rounded-md border border-borderSoft px-8 py-6">
              <Text
                className="text-ink text-[12px]"
                style={{ fontFamily: "ui-monospace", lineHeight: 18 }}
              >
                {permissionPrompt}
              </Text>
            </View>
          </View>
        ) : (
          <Text className="flex-1 text-accent text-[15px] font-bold">{permissionPrompt}</Text>
        )}
      </View>
      <View className="flex-row gap-8">
        <ConfirmButton
          label="Allow"
          tone="ok"
          onPress={() => decide(true)}
          disabled={disabled}
          className="flex-1"
        />
        <ConfirmButton
          label="Deny"
          tone="no"
          onPress={() => decide(false)}
          disabled={disabled}
          className="flex-1"
        />
      </View>
    </View>
  );
}
