// T3.3 contract with T3.2: the slot Chat mounts ABOVE the composer. Reads the
// session snapshot itself (no props) and shows PermissionCard (`permission_prompt`)
// or QuestionCard (`question`) pinned above the composer; renders nothing when
// neither is present. Permission takes priority — the daemon does not surface
// both at once, but if it ever did, an unresolved permission is the more
// urgent decision.
import React from "react";

import { PermissionCard } from "../cards/PermissionCard";
import { QuestionCard } from "../cards/QuestionCard";
import { useSessionCtx } from "../../lib/sessionContext";

export default function CardSlot() {
  const { snapshot, send, setPendingAnswer } = useSessionCtx();

  if (!snapshot) return null;

  if (snapshot.permission_prompt != null) {
    return (
      <PermissionCard
        prompt={snapshot.permission_prompt}
        diff={snapshot.diff}
        promptSeq={snapshot.prompt_seq}
        send={send}
        onQueueAnswer={setPendingAnswer}
      />
    );
  }

  if (snapshot.question != null) {
    return (
      <QuestionCard
        question={snapshot.question}
        options={snapshot.question_options}
        allowOther={snapshot.question_allow_other}
        promptSeq={snapshot.prompt_seq}
        send={send}
        onQueueAnswer={setPendingAnswer}
      />
    );
  }

  return null;
}
