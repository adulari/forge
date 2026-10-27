// The first three steps: the welcome (with "set up everything recommended"), the model sources
// (the one required choice), and safety and cost.
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { checklistFinished, checklistProgress, CREDIT_CHOICES, PERMISSION_CHOICES } from "../../lib/onboarding/flow";
import type { ModelSource, PlanStep, Snapshot } from "../../lib/onboarding/types";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Segmented } from "../ds/Segmented";
import { JobPanel } from "./JobPanel";
import { Disclosure, Hint, SecretField, SourceRow, StatusGlyph, type SourceTone } from "./parts";
import { StepFrame } from "./StepFrame";
import type { Controller } from "./useOnboarding";

export interface StepProps {
  c: Controller;
  snapshot: Snapshot;
  step: PlanStep;
  position: { index: number; total: number };
}

export function WelcomeStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const list = c.checklist;
  const progress = list ? checklistProgress(list) : null;
  const finished = list != null && !c.checklistRunning && checklistFinished(list);
  const hasRecommended = snapshot.recommended.length > 0;
  const live = c.slotJob("checklist");
  // Passing the welcome counts as having seen it, so a later launch resumes after it.
  const passWelcome = (go: () => void) => void c.markDone("welcome").then(go);
  return (
    <StepFrame step={step} snapshot={snapshot} position={position}>
      {list ? (
        <View style={styles.checklist} testID="onboarding-checklist">
          <Text style={[type.section, { color: tokens.ink4 }]}>
            SETTING UP{progress ? `  ${progress.done}/${progress.total}` : ""}
          </Text>
          {list.map((item) => (
            <View key={item.key} style={styles.checkRow}>
              <View style={styles.checkHead}>
                <StatusGlyph status={item.status} />
                <Text style={[type.body, { color: item.status === "pending" ? tokens.ink3 : tokens.ink, flex: 1 }]}>{item.title}</Text>
              </View>
              {item.note && item.status !== "running" && item.status !== "waiting" ? (
                <Text style={[type.sub, { color: item.status === "failed" ? tokens.danger : tokens.ink3, paddingLeft: 22 }]}>{item.note}</Text>
              ) : null}
              {item.status === "running" || item.status === "waiting" ? (
                <View style={styles.checkPanel}>
                  <JobPanel
                    view={live}
                    onInput={(job, text) => void c.sendInput(job, text)}
                    onCancel={(job) => void c.cancel(job)}
                  />
                </View>
              ) : null}
              {item.status === "failed" && item.needsYou ? (
                <View style={styles.checkPanel}>
                  <JobPanel
                    view={live}
                    onPaste={(slot, value) => void c.run({ action: "store_secret", slot, value }, { slot: "checklist-paste", step: "models" })}
                  />
                </View>
              ) : null}
            </View>
          ))}
        </View>
      ) : null}

      {finished ? (
        <Button
          label={snapshot.complete ? "Review and finish" : "Continue"}
          onPress={() => passWelcome(() => (snapshot.complete ? c.goTo(snapshot.plan.length - 1) : c.next()))}
          testID="onboarding-after-checklist"
        />
      ) : hasRecommended || c.checklistRunning ? (
        <Button
          label="Set up everything recommended"
          loading={c.checklistRunning}
          onPress={() => void c.runRecommended()}
          testID="onboarding-recommended"
        />
      ) : (
        <Button label="Continue" onPress={() => passWelcome(c.next)} />
      )}
      {!c.checklistRunning && !finished && hasRecommended ? (
        <Button label="Go step by step" variant="secondary" onPress={() => passWelcome(c.next)} />
      ) : null}

      {hasRecommended ? (
        <Disclosure label="What this does">
          {snapshot.recommended.map((item) => (
            <Hint key={item.title}>{item.title}</Hint>
          ))}
          <Hint>Nothing is sent anywhere and nothing opens to the network. Sign-ins open your browser and wait for you.</Hint>
        </Disclosure>
      ) : null}
      {finished ? null : <Button label="Skip for now" variant="ghost" onPress={() => void c.later()} />}
    </StepFrame>
  );
}

function verification(snapshot: Snapshot, id: string): { label: string; tone: "success" | "danger" } | null {
  const v = snapshot.state.verified[id];
  if (!v) return null;
  if (v.status === "verified") return { label: "Checked", tone: "success" };
  if (v.status === "rejected") return { label: "Refused", tone: "danger" };
  return null;
}

function toneOf(source: ModelSource, snapshot: Snapshot): SourceTone {
  if (snapshot.state.verified[source.id]?.status === "rejected") return "attention";
  if (source.state === "ready") return "ready";
  if (source.state === "needs_login") return "attention";
  return "idle";
}

export function ModelsStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const [keyFor, setKeyFor] = useState<string | null>(null);
  const blocked = snapshot.missing.includes("models");
  // While nothing works, the one sign-in that is a single step away is the page's primary action.
  const lead = blocked ? snapshot.sources.find((s) => s.state === "needs_login")?.id : undefined;
  const variantFor = (id: string) => (id === lead ? "primary" : "secondary");
  const featured = snapshot.sources.filter((s) => s.kind !== "api_key" || s.state === "ready" || s.free_tier);
  const more = snapshot.sources.filter((s) => !featured.includes(s));

  const pasteToken = (slot: "claude_oauth_token", value: string) =>
    c.run({ action: "store_secret", slot, value }, { slot: "claude", step: "models" }).then(() =>
      c.run({ action: "verify", source: "claude-cli" }, { slot: "claude", step: "models" }),
    );

  const renderSource = (source: ModelSource) => {
    const slot = `source:${source.id}`;
    const job = c.slotJob(slot);
    const running = job?.state === "running";
    const badge = verification(snapshot, source.id);
    const check = (
      <Button
        label="Check"
        variant="ghost"
        loading={running}
        onPress={() => void c.run({ action: "verify", source: source.id }, { slot, step: "models" })}
      />
    );
    let actions: React.ReactNode = null;
    switch (source.kind) {
      case "claude_code":
        actions =
          source.state === "not_installed" ? null : source.state === "ready" && snapshot.machine.bridges.find((b) => b.id === "claude-cli")?.token_stored ? (
            check
          ) : (
            <Button
              variant={variantFor(source.id)}
              label={source.state === "ready" ? "Connect Claude Code" : "Reconnect Claude Code"}
              loading={running}
              onPress={() => void c.run({ action: "connect_claude" }, { slot, step: "models" })}
              testID="onboarding-connect-claude"
            />
          );
        break;
      case "codex":
      case "antigravity":
        actions = source.state === "ready" ? check : null;
        break;
      case "codex_oauth":
      case "xai_oauth":
        actions =
          source.state === "ready" ? (
            check
          ) : (
            <Button
              variant="secondary"
              label="Sign in"
              loading={running}
              onPress={() => void c.run({ action: source.kind === "codex_oauth" ? "codex_oauth" : "xai_oauth" }, { slot, step: "models" })}
            />
          );
        break;
      case "api_key":
        actions =
          source.state === "ready" ? (
            check
          ) : (
            <Button label="Add key" variant="secondary" onPress={() => setKeyFor(keyFor === source.id ? null : source.id)} />
          );
        break;
      case "local": {
        const rec = snapshot.machine.local.recommended;
        actions = source.state === "ready" ? check : rec ? (
          <Button
            label={`Download ${rec.label}`}
            variant="secondary"
            loading={running}
            onPress={() => void c.run({ action: "ollama_pull", tag: rec.tag }, { slot, step: "models" })}
          />
        ) : null;
        break;
      }
    }
    return (
      <SourceRow
        key={source.id}
        name={source.name}
        detail={source.detail}
        tone={toneOf(source, snapshot)}
        badges={[
          ...(source.free_tier ? [{ label: "Free tier", tone: "accent" as const }] : []),
          ...(badge ? [badge] : []),
        ]}
        actions={actions}
      >
        {job ? (
          <JobPanel
            view={job}
            onInput={(id, text) => void c.sendInput(id, text)}
            onCancel={(id) => void c.cancel(id)}
            onPaste={source.kind === "claude_code" ? (s, v) => void pasteToken(s as "claude_oauth_token", v) : undefined}
          />
        ) : null}
        {keyFor === source.id ? (
          <SecretField
            label={`${source.name} API key`}
            placeholder="Paste the key"
            actionLabel="Save and check"
            keyUrl={source.key_url}
            busy={running}
            onSubmit={(value) => {
              void c.run({ action: "store_key", provider: source.id, key: value }, { slot, step: "models" });
              setKeyFor(null);
            }}
          />
        ) : null}
      </SourceRow>
    );
  };

  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back}>
      <View style={styles.sources}>{featured.map(renderSource)}</View>
      {more.length > 0 ? <Disclosure label={`More providers (${more.length})`}><View style={styles.sources}>{more.map(renderSource)}</View></Disclosure> : null}
      <Button
        label="Continue"
        disabled={blocked}
        onPress={() => {
          void c.markDone("models").then(c.next);
        }}
        testID="onboarding-models-continue"
      />
      {blocked ? <Text style={[type.sub, { color: tokens.warn }]}>Connect at least one model to continue.</Text> : null}
    </StepFrame>
  );
}

export function BehaviorStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const settings = snapshot.machine.settings;
  const [permission, setPermission] = useState(settings.permission || "default");
  const [credit, setCredit] = useState(settings.credit_mode || "normal");
  const job = c.slotJob("behavior");
  const permissionHelp = PERMISSION_CHOICES.find((p) => p.value === permission)?.example;
  const creditHelp = CREDIT_CHOICES.find((p) => p.value === credit)?.example;
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("behavior").then(c.next)}>
      <View style={styles.field}>
        <Text style={[type.heading, { color: tokens.ink }]}>How careful should Forge be with your files?</Text>
        <Segmented options={PERMISSION_CHOICES.map((p) => ({ value: p.value, label: p.label }))} value={permission} onChange={setPermission} />
        {permissionHelp ? <Hint>{permissionHelp}</Hint> : null}
      </View>
      <View style={styles.field}>
        <Text style={[type.heading, { color: tokens.ink }]}>How should it spend your credits?</Text>
        <Segmented options={CREDIT_CHOICES.map((p) => ({ value: p.value, label: p.label }))} value={credit} onChange={setCredit} />
        {creditHelp ? <Hint>{creditHelp}</Hint> : null}
      </View>
      <Button
        label="Save and continue"
        loading={job?.state === "running"}
        onPress={() => {
          void c.run({ action: "set_behavior", permission, credit_mode: credit }, { slot: "behavior", step: "behavior" }).then((v) => {
            if (v.state === "succeeded") c.next();
          });
        }}
      />
      <JobPanel view={job} quietWhenDone />
    </StepFrame>
  );
}

const styles = StyleSheet.create({
  checklist: { gap: space.space12 },
  checkRow: { gap: space.space8 },
  checkHead: { flexDirection: "row", alignItems: "center", gap: space.space8 },
  checkPanel: { paddingLeft: 22 },
  sources: { gap: space.space8 },
  field: { gap: space.space8 },
});
