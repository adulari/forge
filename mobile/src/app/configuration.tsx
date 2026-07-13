import { router } from "expo-router";
import React, { useCallback, useEffect, useMemo, useState } from "react";
import { Pressable, RefreshControl, StyleSheet, Text, View } from "react-native";

import { DesktopDrillDown } from "../components/fleet/DesktopDrillDown";
import { Card } from "../components/ds/Card";
import { Input } from "../components/ds/Input";
import { ListRow } from "../components/ds/ListRow";
import { Screen } from "../components/ds/Screen";
import { SectionHeader } from "../components/ds/SectionHeader";
import { Segmented } from "../components/ds/Segmented";
import { Switch } from "../components/ds/Switch";
import { type ConfigField } from "../lib/api";
import { useConfig, useUpdateConfig } from "../lib/queries";
import { useTokens } from "../theme/ThemeProvider";
import { space } from "../theme/tokens";
import { type } from "../theme/typography";

type Scope = "user" | "project";

function grouped(fields: ConfigField[]) {
  const result = new Map<string, ConfigField[]>();
  for (const field of fields) {
    const fieldsInGroup = result.get(field.group) ?? [];
    fieldsInGroup.push(field);
    result.set(field.group, fieldsInGroup);
  }
  return [...result.entries()];
}

function ConfigFieldRow({ field, scope }: { field: ConfigField; scope: Scope }) {
  const tokens = useTokens();
  const mutation = useUpdateConfig();
  const [draft, setDraft] = useState(field.value);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    setDraft(field.value);
    setError(null);
    setSaved(false);
  }, [field.key, field.value]);

  const save = useCallback((value: string | undefined) => {
    if (field.field_type === "json" && value != null) {
      try {
        JSON.parse(value);
      } catch {
        setSaved(false);
        setError("Enter valid JSON before saving");
        return;
      }
    }
    setError(null);
    setSaved(false);
    mutation.mutate(
      { key: field.key, value, scope },
      {
        onSuccess: () => {
          setDraft(value ?? field.default);
          setSaved(true);
        },
        onError: (reason) => {
          setDraft(field.value);
          setError(reason instanceof Error ? reason.message : "Could not save this setting");
        },
      },
    );
  }, [field.default, field.field_type, field.key, field.value, mutation, scope]);

  const subtitle = error ?? (saved ? "Saved" : field.help ?? `${field.key} · ${field.source}`);
  if (field.field_type === "bool") {
    return <ListRow title={field.label} subtitle={subtitle} trailing={<Switch value={draft === "true"} onValueChange={(value) => { setDraft(String(value)); save(String(value)); }} accessibilityLabel={field.label} disabled={mutation.isPending} />} hasInteractiveTrailing />;
  }
  if (field.field_type === "list") {
    const values = (() => {
      try {
        return JSON.parse(draft) as string[];
      } catch {
        return [];
      }
    })();
    return <View style={styles.field}><Text style={[type.body, { color: tokens.ink }]}>{field.label}</Text><Text style={[type.sub, { color: error ? tokens.danger : tokens.ink3 }]}>{subtitle}</Text>{values.map((value, index) => <View key={`${field.key}-${index}`} style={styles.listItem}><Input containerStyle={styles.listInput} value={value} onChangeText={(next) => { const updated = [...values]; updated[index] = next; setDraft(JSON.stringify(updated)); }} onEndEditing={(event) => { const updated = [...values]; updated[index] = event.nativeEvent.text; save(JSON.stringify(updated)); }} accessibilityLabel={`${field.label} item ${index + 1}`} autoCapitalize="none" autoCorrect={false} /><Pressable onPress={() => { const next = JSON.stringify(values.filter((_, itemIndex) => itemIndex !== index)); setDraft(next); save(next); }} accessibilityRole="button" accessibilityLabel={`Remove ${field.label} item ${index + 1}`}><Text style={[styles.reset, { color: tokens.danger }]}>Remove</Text></Pressable></View>)}<Pressable onPress={() => { const next = JSON.stringify([...values, ""]); setDraft(next); }} accessibilityRole="button" accessibilityLabel={`Add ${field.label} item`}><Text style={[styles.reset, { color: tokens.accent }]}>Add item</Text></Pressable></View>;
  }
  if (field.field_type === "json") {
    return <View style={styles.field}><Input label={field.label} value={draft} onChangeText={setDraft} onEndEditing={(event) => save(event.nativeEvent.text)} multiline numberOfLines={8} autoCapitalize="none" autoCorrect={false} mono error={error ?? undefined} accessibilityLabel={field.label} /><Text style={[type.sub, { color: tokens.ink3 }]}>{field.help ?? `${field.key} · ${field.source}`}</Text></View>;
  }
  if (field.field_type === "enum") {
    return <View style={styles.field}><Text style={[type.body, { color: tokens.ink }]}>{field.label}</Text><Text style={[type.sub, { color: error ? tokens.danger : tokens.ink3 }]}>{subtitle}</Text><Segmented options={field.options.map((option) => ({ value: option, label: option }))} value={draft} onChange={(value) => { setDraft(value); save(value); }} /></View>;
  }
  return <View style={styles.field}><Input label={field.label} value={draft} onChangeText={setDraft} onEndEditing={(event) => { if (event.nativeEvent.text !== field.value) save(event.nativeEvent.text); }} keyboardType={field.field_type === "int" || field.field_type === "float" ? "decimal-pad" : "default"} autoCapitalize="none" autoCorrect={false} error={error ?? undefined} accessibilityLabel={field.label} /><Text style={[type.sub, { color: tokens.ink3 }]}>{field.help ?? `${field.key} · ${field.source}`}</Text>{field.modified ? <Pressable onPress={() => save(undefined)} accessibilityRole="button" accessibilityLabel={`Reset ${field.label}`}><Text style={[styles.reset, { color: tokens.accent }]}>Reset to default ({field.default || "empty"})</Text></Pressable> : null}</View>;
}

export default function ConfigurationScreen() {
  const tokens = useTokens();
  const query = useConfig();
  const [scope, setScope] = useState<Scope>("user");
  const groups = useMemo(() => grouped(query.data?.fields ?? []), [query.data?.fields]);

  return <DesktopDrillDown><Screen scroll refreshControl={<RefreshControl refreshing={query.isFetching} onRefresh={() => void query.refetch()} />} contentContainerStyle={styles.content}>
    <Pressable onPress={() => router.back()} accessibilityRole="button"><Text style={[styles.back, { color: tokens.accent }]}>‹ Settings</Text></Pressable>
    <Text style={[type.title, { color: tokens.ink }]}>Configuration</Text>
    <Text style={[type.sub, { color: tokens.ink3 }]}>Tune Forge’s effective settings. Choose where edits are saved.</Text>
    <Segmented options={[{ value: "user", label: "Save everywhere" }, { value: "project", label: "Save in project" }]} value={scope} onChange={(value) => setScope(value as Scope)} />
    <Card><Text style={[type.sub, { color: tokens.ink2 }]}>Saved settings apply to new Forge sessions. Restart forge serve to reload daemon-wide behavior.</Text></Card>
    {query.isError ? <Card><Text style={[type.body, { color: tokens.danger }]}>Could not load configuration. Pull to retry.</Text></Card> : null}
    {query.isLoading ? <Card><Text style={[type.body, { color: tokens.ink3 }]}>Loading your effective configuration…</Text></Card> : null}
    {groups.map(([group, fields]) => <View key={group}><SectionHeader>{group}</SectionHeader><Card padded={false}>{fields.map((field, index) => <ConfigFieldRow key={field.key} field={field} scope={scope} />)}</Card></View>)}
  </Screen></DesktopDrillDown>;
}

const styles = StyleSheet.create({
  content: { paddingTop: space.space12, paddingBottom: space.space32, gap: space.space12 },
  back: { fontSize: 15, fontWeight: "600" },
  field: { gap: space.space4, paddingHorizontal: space.space16, paddingVertical: space.space12 },
  listItem: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  listInput: { flex: 1 },
  reset: { fontSize: 13, fontWeight: "600", paddingTop: space.space4 },
});
