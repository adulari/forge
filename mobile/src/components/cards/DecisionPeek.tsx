// DESIGN_SYSTEM.md §6 DecisionPeek: Sheet showing a waiting session's live
// PermissionCard/QuestionCard inline, from a temporary WS attach, for
// approve-without-navigating (FEATURES.md §5, §1.2 `waiting_reason` gap).
//
// Attach/detach lifecycle: `useSessionSocket` is only ever called while this
// component is mounted (gated on `visible && sessionId` by the parent Sheet
// wrapper below). Mounting opens the socket; unmounting runs ws.ts's effect
// cleanup, which tears the socket down — so closing the sheet (scrim tap, Esc,
// swipe-down, or the header close button) detaches it with no separate
// `.close()` call needed. There is no lingering connection once dismissed.
import { AlertTriangle, CircleCheck, X } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { Button } from "../ds/Button";
import { EmptyState } from "../ds/EmptyState";
import { IconButton } from "../ds/IconButton";
import { Sheet } from "../ds/Sheet";
import { Skeleton } from "../ds/Skeleton";
import { useAuth } from "../../lib/auth";
import { useSessionSocket } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { PermissionCard } from "./PermissionCard";
import { QuestionCard } from "./QuestionCard";

export interface DecisionPeekProps {
  sessionId: string | null;
  visible: boolean;
  onClose: () => void;
}

export function DecisionPeek({ sessionId, visible, onClose }: DecisionPeekProps) {
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Decision peek">
      {visible && sessionId ? (
        <DecisionPeekBody sessionId={sessionId} onClose={onClose} />
      ) : (
        // Keeps the sheet's height stable while it animates closed; the socket
        // is already unmounted at this point (visible is false).
        <View style={styles.emptyBody} />
      )}
    </Sheet>
  );
}

function DecisionPeekBody({ sessionId, onClose }: { sessionId: string; onClose: () => void }) {
  const tokens = useTokens();
  const { baseUrl } = useAuth();
  const { snapshot, connectionState, send } = useSessionSocket(baseUrl, sessionId);
  // ws.ts only escalates to "unreachable" after several failed reconnect attempts (~15s of
  // backoff) — before that it's "connecting"/"reconnecting", which reads fine under the
  // skeleton below. Only the escalated state needs to break out of the skeleton, otherwise
  // a normal transient reconnect blip would flash the error message for no reason.
  const unreachable = connectionState === "unreachable";

  return (
    <View style={styles.container}>
      <View style={styles.header}>
        <Text style={[typeScale.heading, { color: tokens.ink }]} numberOfLines={1}>
          needs you
        </Text>
        <Text
          style={[typeScale.meta, styles.sessionId, { color: tokens.ink3, fontFamily: monoFamily.regular }]}
          numberOfLines={1}
        >
          #{sessionId.slice(0, 8)}
        </Text>
        <View style={styles.headerSpacer} />
        <IconButton
          icon={<X size={20} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={onClose}
          accessibilityLabel="Close decision peek"
        />
      </View>

      <View style={styles.body}>
        {snapshot == null && unreachable ? (
          <EmptyState
            icon={AlertTriangle}
            message="server unreachable — can't reach the daemon to peek at this session."
            action={<Button label="Close" variant="secondary" onPress={onClose} />}
          />
        ) : snapshot == null ? (
          <View style={styles.loading}>
            <Skeleton width="60%" height={17} />
            <Skeleton width="100%" height={64} style={styles.loadingGap} />
          </View>
        ) : snapshot.closed ? (
          <EmptyState icon={CircleCheck} message="this session has ended." />
        ) : snapshot.permission_prompt != null ? (
          <PermissionCard
            prompt={snapshot.permission_prompt}
            diff={snapshot.diff}
            promptSeq={snapshot.prompt_seq}
            send={send}
          />
        ) : snapshot.question != null ? (
          <QuestionCard
            question={snapshot.question}
            options={snapshot.question_options}
            allowOther={snapshot.question_allow_other}
            promptSeq={snapshot.prompt_seq}
            send={send}
          />
        ) : (
          <EmptyState
            icon={CircleCheck}
            message="nothing needs you here anymore."
            action={<Button label="Close" variant="secondary" onPress={onClose} />}
          />
        )}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  container: { flex: 1 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    gap: space.space8,
    paddingHorizontal: space.space16,
    paddingBottom: space.space12,
  },
  sessionId: {},
  headerSpacer: { flex: 1 },
  body: { paddingHorizontal: space.space16, paddingBottom: space.space24 },
  loading: {},
  loadingGap: { marginTop: space.space12 },
  emptyBody: { height: 1 },
});
