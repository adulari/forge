import { Download } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import { Button } from "../ds/Button";
import { Chip } from "../ds/Chip";
import { EmptyState } from "../ds/EmptyState";
import { Input } from "../ds/Input";
import { SearchField } from "../ds/SearchField";
import { Segmented } from "../ds/Segmented";
import { Sheet } from "../ds/Sheet";
import { Skeleton } from "../ds/Skeleton";
import { Switch } from "../ds/Switch";
import { useToast } from "../ds/ToastHost";
import { filterAvailable, type SkillScopeChoice } from "../../lib/skillsApi";
import { useAddSkillSource, useAvailableSkills, useInstallSkill, useRemoveSkillSource, useSkillsState } from "../../lib/skillsQueries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";

interface Props {
  visible: boolean;
  onClose: () => void;
  session?: string | null;
}

export function MarketplaceSheet({ visible, onClose, session }: Props) {
  const tokens = useTokens();
  const toast = useToast();
  const state = useSkillsState(session);
  const sources = useMemo(() => state.data?.sources ?? [], [state.data]);
  const [officialOnly, setOfficialOnly] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [scope, setScope] = useState<SkillScopeChoice>("user");
  const [pending, setPending] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const [newSource, setNewSource] = useState("");
  const install = useInstallSkill();
  const addSource = useAddSkillSource();
  const removeSource = useRemoveSkillSource();

  const shown = useMemo(() => (officialOnly ? sources.filter((s) => s.official) : sources), [sources, officialOnly]);
  useEffect(() => {
    if (!shown.some((s) => s.name === selected)) setSelected(shown[0]?.name ?? null);
  }, [shown, selected]);

  const available = useAvailableSkills(visible ? selected : null);
  const rows = useMemo(() => filterAvailable(available.data ?? [], search), [available.data, search]);
  const installedNames = useMemo(() => new Set((state.data?.packs ?? []).filter((p) => p.scope === scope).map((p) => p.name)), [state.data, scope]);

  const doInstall = (name: string) => {
    setPending(name);
    install.mutate(
      { source: name, scope, marketplace: selected ?? undefined, session },
      {
        onSuccess: (result) => toast.show(`Installed ${result.installed.join(", ")}`, { tone: "success" }),
        onError: (error) => toast.show(error.message || "Install failed", { tone: "danger" }),
        onSettled: () => setPending(null),
      },
    );
  };

  const doAddSource = () => {
    addSource.mutate(
      { name: newName.trim(), source: newSource.trim() },
      {
        onSuccess: () => {
          setSelected(newName.trim());
          setNewName("");
          setNewSource("");
        },
        onError: (error) => toast.show(error.message || "Could not add the source", { tone: "danger" }),
      },
    );
  };

  const current = sources.find((s) => s.name === selected);

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Skills marketplace" maxHeightRatio={0.92}>
      <ScrollView contentContainerStyle={styles.body} keyboardShouldPersistTaps="handled">
        <Text style={[type.title, { color: tokens.ink }]}>Skills marketplace</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>Install skills from a git repository. Nothing runs until you install.</Text>
        <View style={styles.row}>
          <Text style={[type.sub, { color: tokens.ink2 }]}>Official only</Text>
          <Switch value={officialOnly} onValueChange={setOfficialOnly} accessibilityLabel="Official sources only" />
        </View>
        <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.chips} accessibilityRole="tablist">
          {shown.map((source) => (
            <Chip key={source.name} label={source.name} selected={source.name === selected} onPress={() => setSelected(source.name)} />
          ))}
        </ScrollView>
        {current ? (
          <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
            {current.source}
            {current.ref ? ` @${current.ref}` : ""}
          </Text>
        ) : null}
        {session ? (
          <Segmented
            options={[
              { value: "user", label: "For me" },
              { value: "project", label: "This project" },
            ]}
            value={scope}
            onChange={setScope}
          />
        ) : null}
        <SearchField value={search} onChangeText={setSearch} placeholder="Search this source" accessibilityLabel="Search marketplace skills" />
        {available.isLoading ? (
          <View style={styles.list}>
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} width="80%" height={14} />
            ))}
          </View>
        ) : available.isError ? (
          <EmptyState icon={Download} message={available.error.message || "Could not read this source."} action={<Button label="Retry" variant="secondary" onPress={() => void available.refetch()} />} />
        ) : rows.length === 0 ? (
          <EmptyState icon={Download} message={search ? "No skills match that search." : "This source lists no skills."} />
        ) : (
          <View style={styles.list}>
            {rows.map((row) => (
              <View key={row.name} style={[styles.item, { borderBottomColor: tokens.hairline }]}>
                <View style={styles.itemText}>
                  <Text style={[styles.itemName, { color: tokens.ink }]} numberOfLines={1}>
                    {row.name}
                  </Text>
                  <Text style={[type.sub, { color: tokens.ink3 }]} numberOfLines={2}>
                    {row.description}
                  </Text>
                </View>
                <Button
                  label={installedNames.has(row.name) ? "Reinstall" : "Install"}
                  variant="secondary"
                  loading={pending === row.name}
                  disabled={pending != null}
                  onPress={() => doInstall(row.name)}
                  accessibilityLabel={`Install ${row.name}`}
                />
              </View>
            ))}
          </View>
        )}
        <View style={[styles.addBox, { borderColor: tokens.border, backgroundColor: tokens.bg2 }]}>
          <Text style={[type.section, { color: tokens.ink4 }]}>Add a source</Text>
          <Input label="Name" value={newName} onChangeText={setNewName} autoCapitalize="none" autoCorrect={false} />
          <Input label="Git URL or owner/repo" value={newSource} onChangeText={setNewSource} autoCapitalize="none" autoCorrect={false} mono />
          <View style={styles.row}>
            <Button label="Add source" variant="secondary" disabled={!newName.trim() || !newSource.trim()} loading={addSource.isPending} onPress={doAddSource} />
            {current && !current.official ? (
              <Button label="Remove source" variant="ghost" onPress={() => removeSource.mutate(current.name)} accessibilityLabel={`Remove source ${current.name}`} />
            ) : null}
          </View>
        </View>
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: { padding: space.space16, gap: space.space12 },
  row: { flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: space.space8 },
  chips: { gap: space.space8, paddingVertical: space.space4 },
  list: { gap: space.space8 },
  item: { flexDirection: "row", alignItems: "center", gap: space.space12, paddingVertical: space.space8, borderBottomWidth: StyleSheet.hairlineWidth },
  itemText: { flex: 1, minWidth: 0, gap: 2 },
  itemName: { fontSize: 14, fontFamily: monoFamily.bold },
  addBox: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius16, padding: space.space12, gap: space.space8 },
});
