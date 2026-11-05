// Why-trace: which agent turn wrote a line. This chunk loads on first use (the toggle in the file
// header); the rows only ever see the plain map it reports.
import { CornerDownRight, History, X } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { BlameLine } from "../../lib/api";
import { useBlame } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, sansFamilyFor } from "../../theme/typography";
import { blameMap, type BlameMap, whenLabel, whySummary } from "./blameModel";
import { useIde } from "./IdeContext";
import { focusTurn } from "./ideSignals";
import { IdeIconButton, IdeTextButton } from "./ideUi";

const EMPTY: BlameMap = new Map();

/** Fetches blame for one file and reports it. Renders nothing. */
export function WhyTraceLoader({ sessionId, path, onMap }: { sessionId: string; path: string; onMap: (map: BlameMap, loading: boolean) => void }) {
  const blame = useBlame(sessionId, path, true);
  useEffect(() => {
    onMap(blame.data ? blameMap(blame.data.lines) : EMPTY, blame.isPending);
    return () => onMap(EMPTY, false);
  }, [blame.data, blame.isPending, onMap]);
  return null;
}

/** The detail for the line the user clicked: agent, turn, the prompt, the tool call. */
export function WhyTraceCard({ info, path, line, onClose }: { info: BlameLine; path: string; line: number; onClose: () => void }) {
  const tokens = useTokens();
  const { sessions, setScope } = useIde();
  const agent = sessions.find((row) => row.id === info.session_id);
  const { title, detail } = whySummary(info);
  const [now] = useState(() => Date.now() / 1000);
  const when = whenLabel(info.created_at, now);
  return (
    <View style={[styles.card, { borderTopColor: tokens.border, backgroundColor: tokens.bg1 }]}>
      <History size={13} strokeWidth={1.75} color={tokens.ink3} />
      <View style={styles.body}>
        <Text numberOfLines={1} style={[styles.title, { color: tokens.ink }]}>
          {`Line ${line} · ${agent?.title?.trim() || `session ${info.session_id.slice(0, 8)}`} · ${title}${when ? ` · ${when}` : ""}`}
        </Text>
        {detail ? (
          <Text numberOfLines={2} style={[styles.detail, { color: tokens.ink2 }]}>
            {detail}
          </Text>
        ) : null}
        {info.tool_call_id ? (
          <Text numberOfLines={1} style={[styles.mono, { color: tokens.ink4 }]}>
            {`tool call ${info.tool_call_id}`}
          </Text>
        ) : null}
      </View>
      <IdeTextButton
        label="Jump to turn"
        icon={<CornerDownRight size={12} strokeWidth={1.75} color={tokens.ink2} />}
        onPress={() => {
          setScope(info.session_id);
          focusTurn(info.session_id, info.turn, path);
        }}
      />
      <IdeIconButton icon={<X size={13} strokeWidth={1.75} color={tokens.ink3} />} label="Close why-trace" onPress={onClose} />
    </View>
  );
}

const styles = StyleSheet.create({
  card: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8, borderTopWidth: StyleSheet.hairlineWidth },
  body: { flex: 1, minWidth: 0, gap: 2 },
  title: { fontFamily: sansFamilyFor(600), fontSize: 11.5 },
  detail: { fontFamily: sansFamilyFor(400), fontSize: 11.5, lineHeight: 15 },
  mono: { fontFamily: monoFamily.regular, fontSize: 10 },
});
