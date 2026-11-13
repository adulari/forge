// "This session is working in <repo>. Move it there?" — shown above the composer while an idle
// Home-scope session keeps `cd`-ing into one repository. Accepting re-roots the session through
// the same switch `/project` uses; dismissing stops the offer for that repo in this session.
// Both answers echo the offered path so a stale tap (the offer changed) is dropped by the host.
import { GitBranch } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { Button } from "../ds/Button";
import { Card } from "../ds/Card";
import { useToast } from "../ds/ToastHost";
import { projectOfferCopy, projectOfferDismiss, projectOfferMove } from "../../lib/projectOffer";
import { type ProjectSuggestion, type RemoteInput } from "../../lib/ws";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface ProjectOfferCardProps {
  suggestion: ProjectSuggestion;
  send: (input: RemoteInput) => boolean;
}

export function ProjectOfferCard({ suggestion, send }: ProjectOfferCardProps) {
  const tokens = useTokens();
  const toast = useToast();
  const [answered, setAnswered] = useState(false);
  const copy = projectOfferCopy(suggestion);

  // The card unmounts when the host clears the offer; a different repo offered later unlocks it.
  useEffect(() => setAnswered(false), [suggestion.path]);

  const answer = (input: RemoteInput) => {
    if (answered) return;
    if (!send(input)) {
      toast.show("not sent — reconnect and try again", { tone: "danger" });
      return;
    }
    setAnswered(true);
  };

  return (
    <Card variant="feature">
      <View style={styles.header}>
        <GitBranch size={15} strokeWidth={1.8} color={tokens.ink2} />
        <Text style={[typeScale.section, { color: tokens.ink2 }]}>project</Text>
      </View>
      <Text style={[typeScale.body, { color: tokens.ink }]}>{copy.title}</Text>
      <Text style={[typeScale.sub, { color: tokens.ink3 }, styles.detail]}>{copy.detail}</Text>
      <View style={styles.actions}>
        <Button
          label="Move there"
          onPress={() => answer(projectOfferMove(suggestion))}
          disabled={answered}
          accessibilityLabel={`Move this session to ${suggestion.name}`}
          testID="project-offer-move"
        />
        <Button
          label="Not now"
          variant="ghost"
          onPress={() => answer(projectOfferDismiss(suggestion))}
          disabled={answered}
          accessibilityLabel={`Do not offer ${suggestion.name} again in this session`}
          testID="project-offer-dismiss"
        />
      </View>
    </Card>
  );
}

const styles = StyleSheet.create({
  header: { flexDirection: "row", alignItems: "center", gap: space.space8, marginBottom: space.space4 },
  detail: { marginTop: space.space4 },
  actions: { flexDirection: "row", gap: space.space8, marginTop: space.space12 },
});
