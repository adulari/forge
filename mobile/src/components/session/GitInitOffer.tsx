// "This folder is not a git repository" with a one-tap `git init` (plus an empty first commit,
// done by the daemon so worktrees have a base to branch from).
import { GitBranch } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { ApiError } from "../../lib/api";
import { useInitProject } from "../../lib/queriesProjects";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";

export function GitInitOffer({ path, onInitialized }: { path: string; onInitialized?: () => void }) {
  const tokens = useTokens();
  const init = useInitProject();
  const error = init.error instanceof ApiError ? init.error.message : init.isError ? "Could not initialize git" : null;

  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
        Not a git repository, so worktrees and diffs are off here.
      </Text>
      <Button
        label="Initialize git here"
        variant="secondary"
        icon={<GitBranch size={15} strokeWidth={1.8} color={tokens.ink2} />}
        loading={init.isPending}
        onPress={() => init.mutate(path, { onSuccess: () => onInitialized?.() })}
        accessibilityLabel={`Initialize git in ${path}`}
      />
      {error ? (
        <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>
          {error}
        </Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { gap: space.space8, alignItems: "flex-start", paddingVertical: space.space8 },
});
