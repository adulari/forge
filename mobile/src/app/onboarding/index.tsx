// Desktop onboarding (docs/features/onboarding.md). Opens by itself on the desktop app until the
// required steps pass: first Forge itself is installed and started (phase 0), then the daemon's
// shared plan is walked here, with "set up everything recommended" as the welcome's one action.
// Left rail plus a content card on a wide window; a top progress bar and one column on a narrow one.
import { router, useLocalSearchParams } from "expo-router";
import { Flame } from "lucide-react-native";
import React, { useMemo } from "react";
import { ActivityIndicator, ScrollView, StyleSheet, Text, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { Button } from "../../components/ds/Button";
import { Stepper, type StepperStep } from "../../components/ds/Stepper";
import { Phase0View } from "../../components/onboarding/Phase0View";
import { BehaviorStep, ModelsStep, WelcomeStep, type StepProps } from "../../components/onboarding/stepsCore";
import {
  AnywhereStep,
  BenchmarksStep,
  DoneStep,
  ImportStep,
  PreferencesStep,
  ReportingStep,
  ServiceStep,
  SurfacesStep,
  WebSearchStep,
} from "../../components/onboarding/stepsMore";
import { useOnboarding, type Controller } from "../../components/onboarding/useOnboarding";
import { railEntries, type Phase0Row } from "../../lib/onboarding/flow";
import type { StepId } from "../../lib/onboarding/types";
import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type } from "../../theme/typography";
import { useBreakpoint } from "../../theme/useBreakpoint";

const STEPS: Partial<Record<StepId, React.ComponentType<StepProps>>> = {
  welcome: WelcomeStep,
  models: ModelsStep,
  behavior: BehaviorStep,
  web_search: WebSearchStep,
  benchmarks: BenchmarksStep,
  import: ImportStep,
  service: ServiceStep,
  anywhere: AnywhereStep,
  surfaces: SurfacesStep,
  preferences: PreferencesStep,
  reporting: ReportingStep,
  done: DoneStep,
};

function phase0Steps(rows: Phase0Row[]): StepperStep[] {
  return rows
    .filter((r) => r.status !== "skipped")
    .map((r) => ({
      key: r.key,
      label: r.title,
      status: r.status === "done" ? "done" : r.status === "pending" ? "todo" : "current",
    }));
}

export default function OnboardingScreen() {
  const params = useLocalSearchParams<{ focus?: string }>();
  const c = useOnboarding({ focus: typeof params.focus === "string" ? params.focus : undefined });
  const tokens = useTokens();
  const { isCompact } = useBreakpoint();

  const railSteps: StepperStep[] = useMemo(() => {
    if (c.phase === "phase0") return phase0Steps(c.phase0);
    if (c.snapshot) return railEntries(c.snapshot, c.index).map((e) => ({ key: e.key, label: e.label, status: e.status, required: e.required, isNew: e.isNew }));
    return [];
  }, [c.phase, c.phase0, c.snapshot, c.index]);

  const body = <Body c={c} />;

  return (
    <SafeAreaView style={[styles.root, { backgroundColor: tokens.bg0 }]} edges={["top", "bottom", "left", "right"]}>
      {isCompact ? (
        <View style={styles.compact}>
          <View style={styles.brandRow}>
            <Flame size={20} strokeWidth={1.75} color={tokens.accent} />
            <Text style={[type.title, { color: tokens.ink }]}>Forge setup</Text>
          </View>
          {railSteps.length > 0 ? (
            <Stepper
              variant="bar"
              steps={railSteps}
              activeIndex={c.phase === "phase0" ? Math.max(0, railSteps.findIndex((s) => s.status === "current")) : c.index}
              testID="onboarding-progress"
            />
          ) : null}
          <ScrollView contentContainerStyle={styles.compactBody} keyboardShouldPersistTaps="handled">
            {body}
          </ScrollView>
        </View>
      ) : (
        <View style={styles.wide}>
          <View style={[styles.rail, { borderRightColor: tokens.border, backgroundColor: tokens.bg1 }]}>
            <View style={styles.brandRow}>
              <Flame size={22} strokeWidth={1.75} color={tokens.accent} />
              <Text style={[type.title, { color: tokens.ink }]}>Forge</Text>
              <Text style={[type.monoMeta, { color: tokens.ink4, fontFamily: monoFamily.regular }]}>setup</Text>
            </View>
            <ScrollView contentContainerStyle={styles.railScroll}>
              <Stepper
                variant="rail"
                steps={railSteps}
                activeIndex={c.index}
                onSelect={c.phase === "daemon" ? (_key, i) => c.goTo(i) : undefined}
                testID="onboarding-rail"
              />
            </ScrollView>
          </View>
          <ScrollView style={styles.main} contentContainerStyle={styles.mainBody} keyboardShouldPersistTaps="handled">
            <View style={styles.column}>{body}</View>
          </ScrollView>
        </View>
      )}
    </SafeAreaView>
  );
}

function Body({ c }: { c: Controller }) {
  const tokens = useTokens();
  if (c.phase === "loading" || (c.phase === "daemon" && !c.snapshot)) {
    return (
      <View style={styles.center} testID="onboarding-loading">
        <ActivityIndicator color={tokens.accent} />
        <Text style={[type.body, { color: tokens.ink2 }]}>Looking at this computer…</Text>
      </View>
    );
  }
  if (c.phase === "phase0") return <Phase0View rows={c.phase0} onRetry={c.retryPhase0} />;
  if (c.phase === "unavailable") {
    return (
      <Notice
        title="Setup runs on the computer Forge is installed on"
        text="This app is connected to Forge through a network address, so it cannot change the setup. Open Forge on that computer to finish setting it up."
        actionLabel="Continue to Forge"
        onAction={() => router.replace("/(tabs)")}
      />
    );
  }
  if (c.phase === "error" || !c.snapshot) {
    return (
      <Notice
        title="Forge did not answer"
        text={c.error ?? "Something went wrong while looking at this computer."}
        actionLabel="Try again"
        onAction={c.retryPhase0}
        secondaryLabel="Connect to a server"
        onSecondary={() => router.replace("/connect")}
      />
    );
  }
  const step = c.snapshot.plan[Math.min(c.index, c.snapshot.plan.length - 1)];
  const View_ = STEPS[step.id];
  if (!View_) return null;
  return <View_ c={c} snapshot={c.snapshot} step={step} position={{ index: c.index, total: c.snapshot.plan.length }} />;
}

function Notice({
  title,
  text,
  actionLabel,
  onAction,
  secondaryLabel,
  onSecondary,
}: {
  title: string;
  text: string;
  actionLabel: string;
  onAction: () => void;
  secondaryLabel?: string;
  onSecondary?: () => void;
}) {
  const tokens = useTokens();
  return (
    <View style={[styles.notice, { backgroundColor: tokens.bg1, borderColor: tokens.border }]}>
      <Text style={[type.title, { color: tokens.ink }]}>{title}</Text>
      <Text style={[type.body, { color: tokens.ink2 }]}>{text}</Text>
      <View style={styles.noticeActions}>
        <Button label={actionLabel} onPress={onAction} />
        {secondaryLabel && onSecondary ? <Button label={secondaryLabel} variant="ghost" onPress={onSecondary} /> : null}
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  root: { flex: 1 },
  wide: { flex: 1, flexDirection: "row" },
  rail: { width: 264, borderRightWidth: StyleSheet.hairlineWidth, paddingTop: space.space24, paddingHorizontal: space.space12, gap: space.space16 },
  railScroll: { paddingBottom: space.space24 },
  brandRow: { flexDirection: "row", alignItems: "center", gap: space.space8, paddingHorizontal: space.space8 },
  main: { flex: 1 },
  mainBody: { alignItems: "center", paddingVertical: space.space32, paddingHorizontal: space.space24 },
  column: { width: "100%", maxWidth: 720 },
  compact: { flex: 1, paddingHorizontal: space.space16, paddingTop: space.space12, gap: space.space12 },
  compactBody: { paddingBottom: space.space32 },
  center: { minHeight: 240, alignItems: "center", justifyContent: "center", gap: space.space12 },
  notice: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius4, padding: space.space24, gap: space.space12 },
  noticeActions: { flexDirection: "row", gap: space.space8, flexWrap: "wrap" },
});
