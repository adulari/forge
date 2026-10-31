// "Worktrees" settings: where session worktrees live, the commands run around creating and
// removing them, and which git-ignored files (.env, editor settings) are copied in. Backed by
// the daemon's `[worktree]` config table; user config only, because setup and cleanup are shell
// commands a cloned project must not be able to plant.
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import type { ConfigField } from "../../lib/api";
import { useUpdateConfig } from "../../lib/queries";
import {
  COPY_IGNORED_SUGGESTIONS,
  commandToSave,
  globProblem,
  normalizeRoot,
  parseGlobList,
  serializeGlobList,
  withGlob,
  withoutGlob,
  type WorktreeRootSetting,
} from "../../lib/worktreeSettings";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { Input } from "../ds/Input";
import { Segmented } from "../ds/Segmented";

const ROOT_OPTIONS: { value: WorktreeRootSetting; label: string }[] = [
  { value: "project", label: "In the project" },
  { value: "global", label: "Shared folder" },
];

const ROOT_HINT: Record<WorktreeRootSetting, string> = {
  project: "Worktrees sit next to the code in <repo>/.forge/worktrees.",
  global: "Worktrees live in Forge's data folder, outside every repository.",
};

function useWorktreeSave() {
  const mutation = useUpdateConfig();
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const save = (key: string, value: string | undefined) => {
    setError(null);
    setSaved(false);
    mutation.mutate(
      { key, value, scope: "user" },
      {
        onSuccess: () => setSaved(true),
        onError: (reason) => setError(reason instanceof Error ? reason.message : "Could not save this setting"),
      },
    );
  };
  return { save, error, saved, pending: mutation.isPending };
}

function CommandField({ field, label, placeholder, hint }: { field: ConfigField; label: string; placeholder: string; hint: string }) {
  const tokens = useTokens();
  const [draft, setDraft] = useState(field.value);
  const { save, error, saved, pending } = useWorktreeSave();
  useEffect(() => setDraft(field.value), [field.value]);
  const dirty = commandToSave(draft) !== commandToSave(field.value);
  return (
    <View style={styles.block}>
      <Input
        label={label}
        value={draft}
        onChangeText={setDraft}
        placeholder={placeholder}
        mono
        autoCapitalize="none"
        autoCorrect={false}
        error={error ?? undefined}
        accessibilityLabel={label}
      />
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{saved && !dirty ? "Saved" : hint}</Text>
      {dirty ? <Button label="Save" variant="secondary" loading={pending} onPress={() => save(field.key, commandToSave(draft))} /> : null}
    </View>
  );
}

function CopyIgnoredField({ field }: { field: ConfigField }) {
  const tokens = useTokens();
  const globs = parseGlobList(field.value);
  const [entry, setEntry] = useState("");
  const { save, error, saved } = useWorktreeSave();
  const problem = globProblem(entry);
  const add = (raw: string) => {
    if (globProblem(raw) || !raw.trim()) return;
    save(field.key, serializeGlobList(withGlob(globs, raw)));
    setEntry("");
  };
  return (
    <View style={styles.block}>
      <Text style={[typeScale.body, { color: tokens.ink }]}>Copy ignored files</Text>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
        Git-ignored files copied from the main checkout into every new worktree. Tracked files are never copied.
      </Text>
      {globs.map((glob, index) => (
        <View key={glob} style={styles.globRow}>
          <Text style={[typeScale.code, styles.globText, { color: tokens.ink }]} numberOfLines={1}>
            {glob}
          </Text>
          <Button
            label="Remove"
            variant="ghost"
            onPress={() => save(field.key, serializeGlobList(withoutGlob(globs, index)))}
            accessibilityLabel={`Remove ${glob}`}
          />
        </View>
      ))}
      <View style={styles.globRow}>
        <Input
          value={entry}
          onChangeText={setEntry}
          onSubmitEditing={() => add(entry)}
          placeholder=".env*"
          mono
          autoCapitalize="none"
          autoCorrect={false}
          error={problem ?? undefined}
          containerStyle={styles.globInput}
          accessibilityLabel="Add a pattern"
        />
        <Button label="Add" variant="secondary" disabled={!entry.trim() || problem !== null} onPress={() => add(entry)} />
      </View>
      <View style={styles.suggestions}>
        {COPY_IGNORED_SUGGESTIONS.filter((item) => !globs.includes(item)).map((item) => (
          <Chip key={item} label={`+ ${item}`} size="compact" onPress={() => add(item)} />
        ))}
      </View>
      {error ? (
        <Text accessibilityRole="alert" style={[typeScale.sub, { color: tokens.danger }]}>
          {error}
        </Text>
      ) : saved ? (
        <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Saved</Text>
      ) : null}
    </View>
  );
}

export function WorktreeSettingsSection({ fields }: { fields: readonly ConfigField[] }) {
  const tokens = useTokens();
  const byKey = (key: string) => fields.find((field) => field.key === key);
  const root = byKey("worktree.root");
  const setup = byKey("worktree.setup");
  const cleanup = byKey("worktree.cleanup");
  const copy = byKey("worktree.copy_ignored");
  const rootSave = useWorktreeSave();
  const [rootDraft, setRootDraft] = useState<WorktreeRootSetting>(normalizeRoot(root?.value ?? ""));
  useEffect(() => setRootDraft(normalizeRoot(root?.value ?? "")), [root?.value]);

  return (
    <View style={styles.wrap}>
      <Text style={[typeScale.sub, { color: tokens.ink3 }]}>
        Applies to every project on this server. Project config cannot set these, because setup and cleanup run shell commands.
      </Text>
      {root ? (
        <View style={styles.block}>
          <Text style={[typeScale.body, { color: tokens.ink }]}>Location</Text>
          <Segmented
            options={ROOT_OPTIONS}
            value={rootDraft}
            onChange={(value) => {
              setRootDraft(value);
              rootSave.save(root.key, value);
            }}
            testID="worktree-root"
          />
          <Text style={[typeScale.sub, { color: rootSave.error ? tokens.danger : tokens.ink3 }]}>
            {rootSave.error ?? ROOT_HINT[rootDraft]}
          </Text>
        </View>
      ) : null}
      {setup ? (
        <CommandField field={setup} label="Setup command" placeholder="npm ci" hint="Runs in the new worktree right after it is created. A failure is reported but keeps the worktree." />
      ) : null}
      {cleanup ? (
        <CommandField field={cleanup} label="Cleanup command" placeholder="docker compose down" hint="Runs in a worktree just before Forge removes it." />
      ) : null}
      {copy ? <CopyIgnoredField field={copy} /> : null}
      {!root && !setup && !cleanup && !copy ? (
        <Text style={[typeScale.body, { color: tokens.ink3 }]}>This server does not support worktree settings yet. Update Forge on the host.</Text>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { paddingHorizontal: space.space16, paddingVertical: space.space12, gap: space.space16 },
  block: { gap: space.space8 },
  globRow: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  globText: { flex: 1 },
  globInput: { flex: 1 },
  suggestions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
});
