// The remaining steps: web search, benchmarks, importing a setup, the background service,
// Forge Anywhere, other places, notifications and theme, error reports and the summary.
import { router } from "expo-router";
import React, { useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { readySources } from "../../lib/onboarding/flow";
import type { Action } from "../../lib/onboarding/types";
import { useTheme, useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type } from "../../theme/typography";
import { Button } from "../ds/Button";
import { Checkbox } from "../ds/Checkbox";
import { Segmented } from "../ds/Segmented";
import { Switch } from "../ds/Switch";
import { JobPanel } from "./JobPanel";
import { Disclosure, Hint, SecretField, StatusGlyph } from "./parts";
import { StepFrame } from "./StepFrame";
import type { StepProps } from "./stepsCore";

function useSaver(c: StepProps["c"], slot: string) {
  const job = c.slotJob(slot);
  return { job, running: job?.state === "running" };
}

export function WebSearchStep({ c, snapshot, step, position }: StepProps) {
  const { job, running } = useSaver(c, "web_search");
  const search = snapshot.machine.search;
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("web_search").then(c.next)}>
      <Button label="Keep keyless search" onPress={() => void c.markDone("web_search").then(c.next)} />
      <Disclosure label="Better results">
        <SecretField
          label="Brave Search key"
          placeholder="Paste the key"
          actionLabel="Save key"
          keyUrl="https://brave.com/search/api/"
          busy={running}
          onSubmit={(value) => void c.run({ action: "store_secret", slot: "brave", value }, { slot: "web_search", step: "web_search" })}
        />
        {search.docker && !search.searxng_running ? (
          <Button
            label="Start a local search engine"
            variant="secondary"
            loading={running}
            onPress={() => void c.run({ action: "start_searxng" }, { slot: "web_search", step: "web_search" })}
          />
        ) : null}
        <JobPanel view={job} quietWhenDone={false} />
      </Disclosure>
    </StepFrame>
  );
}

export function BenchmarksStep({ c, snapshot, step, position }: StepProps) {
  const { job, running } = useSaver(c, "benchmarks");
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("benchmarks").then(c.next)}>
      <Button label="Use the shared data" onPress={() => void c.markDone("benchmarks").then(c.next)} />
      <Hint>Benchmark data: Artificial Analysis, artificialanalysis.ai</Hint>
      <Disclosure label="Advanced: use your own key">
        <SecretField
          label="Artificial Analysis key"
          placeholder="Paste the key"
          actionLabel="Save key"
          keyUrl="https://artificialanalysis.ai/api"
          busy={running}
          onSubmit={(value) => void c.run({ action: "store_secret", slot: "artificial_analysis", value }, { slot: "benchmarks", step: "benchmarks" })}
        />
        <JobPanel view={job} />
      </Disclosure>
    </StepFrame>
  );
}

export function ImportStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const imports = snapshot.machine.imports;
  const tools = [
    { tool: "claude" as const, found: imports.claude_dir, label: "Claude Code commands and skills" },
    { tool: "codex" as const, found: imports.codex_dir, label: "Codex prompts" },
    { tool: "cursor" as const, found: imports.cursor_dir, label: "Cursor rules" },
  ].filter((t) => t.found);
  const [picked, setPicked] = useState<Record<string, boolean>>(() => Object.fromEntries(tools.map((t) => [t.tool, true])));
  const [keybinds, setKeybinds] = useState(imports.claude_dir && !imports.keybinds_active);
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const chosen = tools.filter((t) => picked[t.tool]).length + (keybinds ? 1 : 0);

  const importSelected = async () => {
    setRunning(true);
    setError(null);
    const actions: Action[] = [
      ...tools.filter((t) => picked[t.tool]).map((t) => ({ action: "import_config" as const, tool: t.tool })),
      ...(keybinds ? [{ action: "set_keybind_preset" as const, preset: "claude-code" as const }] : []),
    ];
    let failed: string | null = null;
    for (const action of actions) {
      const view = await c.run(action, { slot: "import" });
      if (view.state === "failed") failed = view.error ?? "That import did not finish.";
    }
    await c.markDone("import");
    setRunning(false);
    if (failed) setError(failed);
    else c.next();
  };

  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("import").then(c.next)}>
      {tools.map((t) => (
        <View key={t.tool} style={styles.checkRow}>
          <Checkbox value={!!picked[t.tool]} onValueChange={(v) => setPicked((p) => ({ ...p, [t.tool]: v }))} accessibilityLabel={`Import ${t.label}`} />
          <Text style={[type.body, { color: tokens.ink }]}>{t.label}</Text>
        </View>
      ))}
      {imports.claude_dir && !imports.keybinds_active ? (
        <View style={styles.checkRow}>
          <Checkbox value={keybinds} onValueChange={setKeybinds} accessibilityLabel="Use Claude Code key bindings" />
          <Text style={[type.body, { color: tokens.ink }]}>Use Claude Code key bindings</Text>
        </View>
      ) : null}
      {imports.mcp_servers > 0 ? (
        <Hint>
          {imports.mcp_servers} MCP server{imports.mcp_servers === 1 ? "" : "s"} found. Run `forge mcp import` in a terminal to choose which ones to bring.
        </Hint>
      ) : null}
      {step.status === "done" ? (
        <>
          <Hint>Already brought over. Importing again only adds what is new.</Hint>
          <Button label="Continue" onPress={c.next} />
        </>
      ) : (
        <Button label={chosen > 0 ? `Import selected (${chosen})` : "Continue"} loading={running} onPress={() => (chosen > 0 ? void importSelected() : void c.skip("import").then(c.next))} />
      )}
      <JobPanel view={c.slotJob("import")} quietWhenDone={!error} />
      {error ? <Text style={[type.sub, { color: tokens.danger }]}>{error}</Text> : null}
    </StepFrame>
  );
}

export function ServiceStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const service = snapshot.machine.service;
  const up = service.installed && service.running;
  const [lan, setLan] = useState(false);
  const { job, running } = useSaver(c, "service");
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back}>
      {up ? (
        <Button label="Continue" onPress={() => void c.markDone("service").then(c.next)} />
      ) : service.supported ? (
        <>
          <View style={styles.checkRow}>
            <Switch value={lan} onValueChange={setLan} accessibilityLabel="Also reachable on my home network" />
            <Text style={[type.body, { color: tokens.ink }]}>Also reachable from my phone on this network</Text>
          </View>
          <Hint>Off by default. When on, Forge listens on your network behind a token and a self-signed certificate.</Hint>
          <Button
            label="Install and start the service"
            loading={running}
            onPress={() =>
              void c.run({ action: "install_service", exposure: lan ? "lan" : "local" }, { slot: "service", step: "service" }).then((v) => {
                if (v.state === "succeeded") c.next();
              })
            }
            testID="onboarding-install-service"
          />
        </>
      ) : (
        <Text style={[type.body, { color: tokens.warn }]}>This system cannot run Forge as a background service.</Text>
      )}
      <JobPanel view={job} quietWhenDone />
    </StepFrame>
  );
}

export function AnywhereStep({ c, snapshot, step, position }: StepProps) {
  const signedIn = snapshot.machine.anywhere.signed_in;
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("anywhere").then(c.next)} skipLabel="Not now">
      {signedIn ? (
        <Button label="Continue" onPress={() => void c.markDone("anywhere").then(c.next)} />
      ) : (
        <>
          <Button label="Set up Forge Anywhere" onPress={() => router.push("/anywhere")} />
          <Hint>Sessions stay on this computer and are encrypted end to end. You can set this up later in Settings.</Hint>
        </>
      )}
    </StepFrame>
  );
}

export function SurfacesStep({ c, snapshot, step, position }: StepProps) {
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("surfaces").then(c.next)} skipLabel="Not now">
      <Button label="Pair a phone" onPress={() => router.push(snapshot.machine.anywhere.signed_in ? "/anywhere/hosts" : "/anywhere")} />
      <Hint>The Forge app for iPhone and Android connects through Forge Anywhere, or directly on your home network.</Hint>
    </StepFrame>
  );
}

export function PreferencesStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const theme = useTheme();
  const settings = snapshot.machine.settings;
  const [notifications, setNotifications] = useState(settings.notifications);
  // The app's own appearance setting is what the person sees, so it leads; the daemon's copy
  // (used by the terminal) follows it.
  const [scheme, setScheme] = useState<"auto" | "dark" | "light">(
    theme.preference === "system" ? "auto" : theme.preference,
  );
  const [saving, setSaving] = useState(false);
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("preferences").then(c.next)}>
      <View style={styles.checkRow}>
        <Switch value={notifications} onValueChange={setNotifications} accessibilityLabel="Notify me when a long turn ends" />
        <Text style={[type.body, { color: tokens.ink }]}>Tell me when a long turn ends or Forge needs a decision</Text>
      </View>
      <View style={styles.field}>
        <Text style={[type.heading, { color: tokens.ink }]}>Theme</Text>
        <Segmented
          options={[
            { value: "auto", label: "Match system" },
            { value: "dark", label: "Dark" },
            { value: "light", label: "Light" },
          ]}
          value={scheme}
          onChange={(v) => {
            setScheme(v);
            theme.setScheme(v === "auto" ? "system" : v);
          }}
        />
      </View>
      <Button
        label="Save and continue"
        loading={saving}
        onPress={() => {
          setSaving(true);
          void c
            .run({ action: "set_notifications", enabled: notifications }, { slot: "preferences" })
            .then(() => c.run({ action: "set_theme", theme: scheme }, { slot: "preferences", step: "preferences" }))
            .then(() => {
              setSaving(false);
              c.next();
            });
        }}
      />
    </StepFrame>
  );
}

const REPORT_CHOICES = [
  { value: "auto", label: "Automatic", example: "Forge sends a scrubbed report when it hits a bug. Never your prompts, code, paths or secrets." },
  { value: "ask", label: "Ask each time", example: "Forge shows you the report first and sends it only if you say yes." },
  { value: "off", label: "Off", example: "Nothing is ever sent." },
] as const;

export function ReportingStep({ c, snapshot, step, position }: StepProps) {
  const [mode, setMode] = useState<"auto" | "ask" | "off">((snapshot.machine.settings.reporting as "auto" | "ask" | "off") || "auto");
  const { job, running } = useSaver(c, "reporting");
  const help = REPORT_CHOICES.find((o) => o.value === mode)?.example;
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} onSkip={() => void c.skip("reporting").then(c.next)}>
      <Segmented options={REPORT_CHOICES.map((o) => ({ value: o.value, label: o.label }))} value={mode} onChange={setMode} />
      {help ? <Hint>{help}</Hint> : null}
      <Button
        label="Save and continue"
        loading={running}
        onPress={() =>
          void c.run({ action: "set_reporting", mode }, { slot: "reporting", step: "reporting" }).then((v) => {
            if (v.state === "succeeded") c.next();
          })
        }
      />
      <JobPanel view={job} quietWhenDone />
    </StepFrame>
  );
}

export function DoneStep({ c, snapshot, step, position }: StepProps) {
  const tokens = useTokens();
  const ready = readySources(snapshot);
  const machine = snapshot.machine;
  const rows: { ok: boolean; text: string }[] = [
    { ok: ready.length > 0, text: ready.length > 0 ? `Models: ${ready.join(", ")}` : "No model connected yet" },
    { ok: machine.service.installed && machine.service.running, text: machine.service.running ? "Background service running" : "Background service not running" },
    { ok: machine.anywhere.signed_in, text: machine.anywhere.signed_in ? "Forge Anywhere connected" : "Forge Anywhere not set up (optional)" },
  ];
  const missing = snapshot.missing.filter((id) => id !== "done");
  const firstMissing = missing[0];
  return (
    <StepFrame step={step} snapshot={snapshot} position={position} onBack={c.back} detected={[]}>
      <View style={styles.summary}>
        {rows.map((row) => (
          <View key={row.text} style={styles.checkRow}>
            <StatusGlyph status={row.ok ? "done" : "skipped"} />
            <Text style={[type.body, { color: row.ok ? tokens.ink : tokens.ink3 }]}>{row.text}</Text>
          </View>
        ))}
      </View>
      {firstMissing ? (
        <>
          <Text style={[type.body, { color: tokens.warn }]}>
            Forge still needs {missing.map((id) => snapshot.plan.find((s) => s.id === id)?.title ?? id).join(" and ")} before it is ready.
          </Text>
          <Button
            label="Finish the required steps"
            onPress={() => c.goTo(Math.max(0, snapshot.plan.findIndex((s) => s.id === firstMissing)))}
            testID="onboarding-finish"
          />
          <Button label="Open Forge anyway" variant="secondary" onPress={() => void c.later()} />
        </>
      ) : (
        <>
          <Button label="Start a session" onPress={() => void c.finish("/new-session")} testID="onboarding-finish" />
          <Button label="Open Forge" variant="secondary" onPress={() => void c.finish()} />
        </>
      )}
    </StepFrame>
  );
}

const styles = StyleSheet.create({
  checkRow: { flexDirection: "row", alignItems: "center", gap: space.space12 },
  field: { gap: space.space8 },
  summary: { gap: space.space8 },
});
