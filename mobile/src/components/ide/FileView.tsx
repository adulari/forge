// A file tab: the light editor, loaded on first use.
import React, { Suspense } from "react";
import { View } from "react-native";

const FileEditor = React.lazy(() => import("./FileEditor").then((m) => ({ default: m.FileEditor })));

export function FileView({ sessionId, path }: { sessionId: string; path: string }) {
  return (
    <Suspense fallback={<View style={{ flex: 1 }} />}>
      <FileEditor sessionId={sessionId} path={path} />
    </Suspense>
  );
}
