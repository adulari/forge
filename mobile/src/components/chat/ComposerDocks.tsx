// The docks that sit on the composer: the task plan and the session's spend. Their order, where
// the plan lives (here, or only in the side panel on a wide window), what is folded and what was
// hidden for this chat are the person's to set; all of it persists on this device.
import { EyeOff } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import {
  type DockId,
  dismissDock,
  isDismissed,
  reorderDock,
  restoreDock,
  sessionDockState,
  setDockCollapsed,
  setDockPlacement,
  taskSignature,
  useDockPrefs,
} from "../../lib/composerDocks";
import { useSessionSpend } from "../../lib/queries";
import { hasSpend } from "../../lib/spendDock";
import type { Snapshot } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";
import type { MenuEntry } from "../shell/PopoverMenu";
import { MiniPressable } from "./MiniPressable";
import { SpendDock } from "./SpendDock";
import { TaskDock } from "./TaskDock";

const SPEND_SIGNATURE = "spend";
const TITLES: Record<DockId, string> = { tasks: "plan", spend: "spend" };

export interface ComposerDocksProps {
  sessionId: string;
  snapshot: Snapshot | null;
}

export function ComposerDocks({ sessionId, snapshot }: ComposerDocksProps) {
  const tokens = useTokens();
  const prefs = useDockPrefs();
  const { isExpanded } = useBreakpoint();
  const session = sessionDockState(prefs, sessionId);
  const tasks = snapshot?.tasks ?? [];
  const taskSig = taskSignature(tasks);
  const spend = useSessionSpend(sessionId, String(snapshot?.cost_usd ?? 0)).data;

  const planInPanel = prefs.placement === "panel" && isExpanded;
  const visible: Record<DockId, boolean> = {
    tasks: tasks.length > 0 && !planInPanel && !isDismissed(session, "tasks", taskSig),
    spend: hasSpend(spend?.total) && !isDismissed(session, "spend", SPEND_SIGNATURE),
  };
  const shown = prefs.order.filter((id) => visible[id]);

  const hidden: DockId[] = [];
  if (tasks.length > 0 && !planInPanel && isDismissed(session, "tasks", taskSig)) hidden.push("tasks");
  if (hasSpend(spend?.total) && isDismissed(session, "spend", SPEND_SIGNATURE)) hidden.push("spend");

  const menuFor = (id: DockId): MenuEntry[] => {
    const position = shown.indexOf(id);
    const entries: MenuEntry[] = [
      { kind: "item", id: "up", label: "Move up", disabled: position <= 0, onPress: () => void reorderDock(id, -1) },
      { kind: "item", id: "down", label: "Move down", disabled: position < 0 || position >= shown.length - 1, onPress: () => void reorderDock(id, 1) },
    ];
    if (id === "tasks" && isExpanded) {
      entries.push({ kind: "item", id: "panel", label: "Show in the side panel", onPress: () => void setDockPlacement("panel") });
    }
    entries.push(
      { kind: "separator", id: "sep" },
      {
        kind: "item",
        id: "hide",
        label: "Hide for this chat",
        onPress: () => void dismissDock(sessionId, id, id === "tasks" ? taskSig : SPEND_SIGNATURE),
      },
    );
    return entries;
  };

  const toggle = (id: DockId) => () => void setDockCollapsed(sessionId, id, !session.collapsed.includes(id));

  if (shown.length === 0 && hidden.length === 0) return null;
  return (
    <View testID="composer-docks">
      {shown.map((id) =>
        id === "tasks" ? (
          <TaskDock
            key={id}
            tasks={tasks}
            busy={snapshot?.busy ?? false}
            collapsed={session.collapsed.includes(id)}
            onToggle={toggle(id)}
            menu={menuFor(id)}
          />
        ) : (
          <SpendDock key={id} spend={spend} collapsed={session.collapsed.includes(id)} onToggle={toggle(id)} menu={menuFor(id)} />
        ),
      )}
      {hidden.length > 0 ? (
        <View style={styles.restore}>
          {hidden.map((id) => (
            <MiniPressable
              key={id}
              onPress={() => void restoreDock(sessionId, id)}
              accessibilityRole="button"
              accessibilityLabel={`Show the ${TITLES[id]} dock`}
              style={styles.restoreButton}
              testID={`restore-${id}-dock`}
            >
              <EyeOff size={11} strokeWidth={1.75} color={tokens.ink4} />
              <Text style={[typeScale.meta, { color: tokens.ink3 }]}>{`Show ${TITLES[id]}`}</Text>
            </MiniPressable>
          ))}
        </View>
      ) : null}
    </View>
  );
}


const styles = StyleSheet.create({
  restore: { flexDirection: "row", justifyContent: "flex-end", gap: space.space8, paddingHorizontal: space.space12, paddingBottom: space.space4 },
  restoreButton: { flexDirection: "row", alignItems: "center", gap: space.space4, paddingHorizontal: space.space8, paddingVertical: space.space2, borderRadius: 4 },
});
