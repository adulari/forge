// How code outside the agent panel talks to the agent: the panel owns the one session socket and
// registers a sender for as long as that agent is shown. A message for an agent that is not shown
// yet waits and goes out the moment its panel mounts. A module slot, not context, so review
// actions never re-render the panel.
export type AgentSender = (text: string) => boolean;

const senders = new Map<string, AgentSender>();
const queued = new Map<string, string[]>();

export function registerAgentSender(sessionId: string, sender: AgentSender): () => void {
  senders.set(sessionId, sender);
  const waiting = queued.get(sessionId);
  if (waiting && waiting.length > 0) {
    queued.delete(sessionId);
    for (const text of waiting) {
      if (!sender(text)) {
        queued.set(sessionId, [...(queued.get(sessionId) ?? []), text]);
      }
    }
  }
  return () => {
    if (senders.get(sessionId) === sender) senders.delete(sessionId);
  };
}

/** "sent" when the panel for that agent took it; "queued" when it will go once the panel shows it. */
export function sendToAgent(sessionId: string, text: string): "sent" | "queued" {
  const sender = senders.get(sessionId);
  if (sender?.(text)) return "sent";
  queued.set(sessionId, [...(queued.get(sessionId) ?? []), text]);
  return "queued";
}

export function resetAgentBridgeForTests(): void {
  senders.clear();
  queued.clear();
}
