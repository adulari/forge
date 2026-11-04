// Read-only file view on CodeView. Light editing arrives with the editor in the next change.
import { FileText } from "lucide-react-native";
import React from "react";
import { StyleSheet, View } from "react-native";

import { useWorkspaceFile } from "../../lib/queries";
import { space } from "../../theme/tokens";
import { EmptyState } from "../ds/EmptyState";
import { Skeleton } from "../ds/Skeleton";
import { CodeView } from "../workspace/CodeView";
import { highlightLanguage } from "../workspace/fileKind";

const MAX_BYTES = 1_000_000;

export function FileView({ sessionId, path }: { sessionId: string; path: string }) {
  const file = useWorkspaceFile(sessionId, path);
  if (file.isPending) {
    return (
      <View style={styles.skeleton}>
        {[60, 42, 74, 38, 55, 66].map((w, i) => (
          <Skeleton key={i} width={`${w}%`} height={10} />
        ))}
      </View>
    );
  }
  if (file.error || !file.data) {
    return <EmptyState icon={FileText} message={`Could not open this file. ${file.error?.message ?? ""}`.trim()} />;
  }
  if (file.data.size > MAX_BYTES) {
    return <EmptyState icon={FileText} message="This file is too large to show here." />;
  }
  return <CodeView code={file.data.content} language={highlightLanguage(path)} />;
}

const styles = StyleSheet.create({
  skeleton: { padding: space.space16, gap: space.space12 },
});
