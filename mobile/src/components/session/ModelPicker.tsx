import { Check, ChevronDown, Cpu } from "lucide-react-native";
import React, { useEffect, useMemo, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, View } from "react-native";

import type { ModelRow } from "../../lib/api";
import { useModels } from "../../lib/queries";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Badge } from "../ds/Badge";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { SearchField } from "../ds/SearchField";
import { Sheet } from "../ds/Sheet";

interface CatalogModel {
  provider: string;
  model: ModelRow;
}

export interface ModelPickerProps {
  value: string;
  onChange: (model: string) => void;
}

export function ModelPicker({ value, onChange }: ModelPickerProps) {
  const tokens = useTokens();
  const query = useModels();
  const [visible, setVisible] = useState(false);
  const [search, setSearch] = useState("");
  const [manualModel, setManualModel] = useState(value);

  useEffect(() => {
    if (visible) setManualModel(value);
  }, [value, visible]);

  const catalog = useMemo<CatalogModel[]>(
    () =>
      (query.data?.providers ?? [])
        .flatMap(({ provider, models }) => models.map((model) => ({ provider, model })))
        .sort(
          (a, b) =>
            Number(a.model.health != null) - Number(b.model.health != null) ||
            (b.model.benchmark_intelligence ?? -Infinity) -
              (a.model.benchmark_intelligence ?? -Infinity) ||
            a.model.name.localeCompare(b.model.name),
        ),
    [query.data?.providers],
  );

  const selected = catalog.find(({ model }) => model.id === value)?.model;
  const needle = search.trim().toLocaleLowerCase();
  const filtered = catalog.filter(({ provider, model }) =>
    `${model.name} ${model.id} ${provider}`.toLocaleLowerCase().includes(needle),
  );

  const close = () => {
    setVisible(false);
    setSearch("");
  };
  const select = (model: string) => {
    onChange(model);
    close();
  };

  return (
    <>
      <View>
        <Text style={[typeScale.meta, styles.label, { color: tokens.ink3 }]}>Model (optional)</Text>
        <Pressable
          onPress={() => setVisible(true)}
          accessibilityRole="button"
          accessibilityLabel="Choose model"
          accessibilityValue={{ text: (selected?.name ?? value) || "Automatic" }}
          style={({ pressed }) => [
            styles.trigger,
            {
              backgroundColor: tokens.bg2,
              borderColor: pressed ? tokens.borderStrong : tokens.border,
            },
          ]}
        >
          <View style={styles.triggerText}>
            <Text style={[typeScale.body, { color: tokens.ink }]} numberOfLines={1}>
              {(selected?.name ?? value) || "Automatic"}
            </Text>
            <Text style={[typeScale.meta, { color: tokens.ink3 }]} numberOfLines={1}>
              {value ? selected?.id ?? value : "Let Forge choose the best available model"}
            </Text>
          </View>
          <ChevronDown size={18} strokeWidth={1.75} color={tokens.ink3} />
        </Pressable>
      </View>

      <Sheet visible={visible} onClose={close} accessibilityLabel="Choose a model" snapPoints={[0.9]}>
        <View style={styles.sheetContent}>
          <View style={styles.sheetTitle}>
            <Cpu size={20} strokeWidth={1.75} color={tokens.accent} />
            <Text style={[typeScale.heading, { color: tokens.ink }]}>Choose a model</Text>
          </View>
          <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Healthy models are shown first. You can also let Forge route automatically.</Text>

          <SearchField
            value={search}
            onChangeText={setSearch}
            placeholder="Search models or providers"
            accessibilityLabel="Search models"
          />

          <View style={styles.options} accessibilityRole="radiogroup" accessibilityLabel="Available models">
            <ModelOption
              title="Automatic"
              subtitle="Forge chooses the best available model"
              selected={!value}
              onPress={() => select("")}
            />
            {query.isLoading ? (
              <View style={styles.statusRow}>
                <ActivityIndicator color={tokens.accent} />
                <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Loading model catalog…</Text>
              </View>
            ) : null}
            {!query.isLoading && filtered.length === 0 ? (
              <Text style={[typeScale.sub, styles.empty, { color: tokens.ink3 }]}>
                {catalog.length === 0
                  ? "No model catalog is available from this Forge host."
                  : "No models match this search."}
              </Text>
            ) : null}
            {filtered.map(({ provider, model }) => (
              <ModelOption
                key={`${provider}:${model.id}`}
                title={model.name}
                subtitle={`${provider} · ${model.id}`}
                selected={value === model.id}
                health={model.health ? "benched" : "ready"}
                onPress={() => select(model.id)}
              />
            ))}
          </View>

          <View style={[styles.manual, { borderTopColor: tokens.border }]}>
            <Text style={[typeScale.bodyBold, { color: tokens.ink }]}>Use a model ID manually</Text>
            <Text style={[typeScale.sub, { color: tokens.ink3 }]}>Available even when catalog discovery is offline.</Text>
            <Input
              mono
              value={manualModel}
              onChangeText={setManualModel}
              placeholder="provider/model-id"
              autoCapitalize="none"
              autoCorrect={false}
              returnKeyType="done"
              onSubmitEditing={() => {
                if (manualModel.trim()) select(manualModel.trim());
              }}
              accessibilityLabel="Manual model ID"
            />
            <Button
              label="Use model ID"
              onPress={() => select(manualModel.trim())}
              disabled={!manualModel.trim()}
              fullWidth
            />
          </View>
        </View>
      </Sheet>
    </>
  );
}

function ModelOption({ title, subtitle, selected, health, onPress }: { title: string; subtitle: string; selected: boolean; health?: "ready" | "benched"; onPress: () => void }) {
  const tokens = useTokens();
  return (
    <Pressable
      onPress={onPress}
      accessibilityRole="radio"
      accessibilityState={{ checked: selected }}
      accessibilityLabel={`${title}, ${subtitle}`}
      style={[
        styles.option,
        {
          backgroundColor: selected ? tokens.selection : tokens.bg2,
          borderColor: selected ? tokens.accent : tokens.border,
        },
      ]}
    >
      <View style={styles.optionText}>
        <View style={styles.optionTitle}>
          <Text style={[typeScale.body, styles.optionName, { color: tokens.ink }]} numberOfLines={1}>{title}</Text>
          {health ? <Badge label={health} tone={health === "ready" ? "success" : "danger"} /> : null}
        </View>
        <Text style={[typeScale.meta, { color: tokens.ink3, fontFamily: monoFamily.regular }]} numberOfLines={1}>{subtitle}</Text>
      </View>
      {selected ? <Check size={18} strokeWidth={2} color={tokens.accent} /> : null}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  label: { marginBottom: space.space4 },
  trigger: { minHeight: 52, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8, borderWidth: 1, borderRadius: radii.radius8 },
  triggerText: { flex: 1, gap: 2 },
  sheetContent: { paddingHorizontal: space.space16, paddingBottom: space.space24, gap: space.space12 },
  sheetTitle: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  options: { gap: space.space8 },
  option: { minHeight: 56, flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space12, paddingVertical: space.space8, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8 },
  optionText: { flex: 1, gap: 2 },
  optionTitle: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  optionName: { flex: 1 },
  statusRow: { flexDirection: "row", alignItems: "center", justifyContent: "center", gap: space.space8, padding: space.space16 },
  empty: { padding: space.space16, textAlign: "center" },
  manual: { gap: space.space8, paddingTop: space.space16, borderTopWidth: StyleSheet.hairlineWidth },
});
