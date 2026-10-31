import { Play, Square } from "lucide-react-native";
import React from "react";

import { useProjectActions } from "../../lib/queriesProjectActions";
import { useTokens } from "../../theme/ThemeProvider";
import { ListRow } from "../ds/ListRow";
import { UNTRUSTED_HINT } from "./portsModel";
import { useActionControls } from "./useActionControls";

/** Run / Stop rows for a session's project actions, for sheets that list session actions. */
export function SessionActionRows({ sessionId, onDone }: { sessionId: string; onDone: () => void }) {
  const tokens = useTokens();
  const { data } = useProjectActions(sessionId, true, false);
  const controls = useActionControls();
  if (!controls.available || !data) return null;
  if (data.actions.length === 0 && data.untrusted) {
    return <ListRow title="Project actions are off" subtitle={UNTRUSTED_HINT} disabled showSeparator={false} />;
  }
  return (
    <>
      {data.actions.map((action) => (
        <ListRow
          key={action.id}
          title={action.running ? `Stop ${action.name}` : `Run ${action.name}`}
          subtitle={action.command}
          leading={
            action.running ? (
              <Square size={20} strokeWidth={1.75} color={tokens.danger} />
            ) : (
              <Play size={20} strokeWidth={1.75} color={tokens.ink2} />
            )
          }
          onPress={() => {
            if (action.running) controls.stop(sessionId, action);
            else controls.run(sessionId, action);
            onDone();
          }}
        />
      ))}
    </>
  );
}
