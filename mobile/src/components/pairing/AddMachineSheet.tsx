// Add another machine to the fleet over SSH. The paired daemon runs `forge host add` (install,
// start the service, pair it to Forge Anywhere) and this sheet follows the job: progress lines,
// then a safety-code approval before the new device gets account access.
import * as Clipboard from "expo-clipboard";
import { Copy } from "lucide-react-native";
import React, { useCallback, useEffect, useRef, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";

import {
  answerAddHostApproval,
  canAddMachineVia,
  getAddHostJob,
  hostAddCommand,
  isTerminalPhase,
  isValidHostTarget,
  jobToResume,
  listAddHostJobs,
  nextPollDelay,
  startAddHost,
  type AddHostJob,
} from "../../lib/hostAdd";
import { useAuth } from "../../lib/auth";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";
import { Banner } from "../ds/Banner";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";


function message(err: unknown): string {
  return err instanceof Error ? err.message : "something went wrong";
}

export function AddMachineSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const tokens = useTokens();
  const toast = useToast();
  const { baseUrl, servers, activeServerId } = useAuth();
  const transport = servers.find((server) => server.id === activeServerId)?.transport;
  const [target, setTarget] = useState("");
  const [name, setName] = useState("");
  const [jobId, setJobId] = useState<string | null>(null);
  const [job, setJob] = useState<AddHostJob | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const scroller = useRef<ScrollView>(null);

  const reset = useCallback(() => {
    setJobId(null);
    setJob(null);
    setError(null);
    setBusy(false);
  }, []);

  useEffect(() => {
    if (!visible) reset();
  }, [visible, reset]);

  // A sheet closed mid-install leaves its job running on the daemon: pick it back up on open.
  useEffect(() => {
    if (!visible || !baseUrl) return;
    let cancelled = false;
    void listAddHostJobs(baseUrl)
      .then(({ jobs }) => {
        const running = jobToResume(jobs);
        if (!cancelled && running) setJobId((current) => current ?? running.id);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [visible, baseUrl]);

  useEffect(() => {
    if (!jobId || !baseUrl) return;
    let cancelled = false;
    let failures = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const tick = async () => {
      try {
        const next = await getAddHostJob(baseUrl, jobId);
        if (cancelled) return;
        failures = 0;
        setError(null);
        setJob(next);
        if (!isTerminalPhase(next.phase)) timer = setTimeout(() => void tick(), nextPollDelay(0));
      } catch (err) {
        if (cancelled) return;
        // One dropped request must not freeze the sheet on a stale Approve button.
        failures += 1;
        setError(`${message(err)}; retrying`);
        timer = setTimeout(() => void tick(), nextPollDelay(failures));
      }
    };
    void tick();
    return () => {
      cancelled = true;
      if (timer) clearTimeout(timer);
    };
  }, [jobId, baseUrl]);

  const start = async () => {
    if (!baseUrl || !isValidHostTarget(target)) return;
    setBusy(true);
    setError(null);
    try {
      const started = await startAddHost(baseUrl, { target: target.trim(), ...(name.trim() ? { name: name.trim() } : {}) });
      setJobId(started.job_id);
    } catch (err) {
      // 409: the same machine is already being added (for example from a sheet that was closed).
      const running = await listAddHostJobs(baseUrl)
        .then(({ jobs }) => jobToResume(jobs, target.trim()))
        .catch(() => null);
      if (running) setJobId(running.id);
      else setError(message(err));
    } finally {
      setBusy(false);
    }
  };

  const answer = async (approve: boolean) => {
    if (!baseUrl || !jobId) return;
    try {
      await answerAddHostApproval(baseUrl, jobId, approve);
    } catch (err) {
      setError(message(err));
    }
  };

  const copyCommand = async () => {
    try {
      await Clipboard.setStringAsync(hostAddCommand(target, name));
      toast.show("Command copied", { tone: "success" });
    } catch {
      toast.show("Could not copy", { tone: "danger" });
    }
  };

  const relayOnly = !canAddMachineVia(transport);

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Add a machine over SSH" maxHeightRatio={0.92}>
      <View style={styles.body}>
        <Text style={[typeScale.title, { color: tokens.ink }]}>Add a machine</Text>
        <Text style={[typeScale.sub, { color: tokens.ink2 }]}>
          Forge connects to it over SSH from the machine you are paired with, installs itself, starts the service
          and pairs it to your fleet. SSH uses that machine&apos;s own keys; no password is asked for.
        </Text>

        {relayOnly ? (
          <Banner
            tone="warn"
            message="This server is reached over the relay, which cannot start an SSH session. Run the command below in a terminal on a machine that can ssh to it, or pair this server directly first."
          />
        ) : null}

        {!job ? (
          <View style={styles.group}>
            <Input
              label="Host"
              mono
              value={target}
              onChangeText={setTarget}
              placeholder="user@host or an ssh config alias"
              autoCapitalize="none"
              autoCorrect={false}
              accessibilityLabel="SSH host"
              error={target.trim() && !isValidHostTarget(target) ? "Letters, digits and . _ - @ : only" : undefined}
            />
            <Input
              label="Name (optional)"
              value={name}
              onChangeText={setName}
              placeholder="shown in the fleet"
              autoCapitalize="none"
              accessibilityLabel="Machine name"
            />
            <Button
              label="Install and pair"
              onPress={() => void start()}
              disabled={relayOnly || !isValidHostTarget(target)}
              loading={busy}
              fullWidth
            />
            <Button
              label="Copy as a terminal command"
              variant="ghost"
              icon={<Copy size={16} strokeWidth={1.75} color={tokens.ink} />}
              onPress={() => void copyCommand()}
              fullWidth
            />
          </View>
        ) : (
          <View style={styles.group}>
            <Text style={[typeScale.bodyBold, { color: tokens.ink }]} accessibilityLiveRegion="polite">
              {job.phase === "failed" ? "Could not finish" : job.phase === "done" ? "Ready" : job.step}
            </Text>
            <ScrollView
              ref={scroller}
              style={[styles.log, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
              onContentSizeChange={() => scroller.current?.scrollToEnd({ animated: false })}
              accessibilityLabel="Progress"
            >
              {job.lines.map((line, index) => (
                <Text key={index} style={[typeScale.monoMeta, { color: tokens.ink3, fontFamily: monoFamily.regular }]}>
                  {line}
                </Text>
              ))}
            </ScrollView>
            {job.phase === "awaiting_approval" ? (
              <View style={styles.group}>
                <Banner
                  tone="warn"
                  message={
                    job.safety_code
                      ? `This device computed the safety code ${job.safety_code}. Approve only if ${job.target} shows exactly the same code${job.remote_safety_code ? ` (it printed ${job.remote_safety_code})` : ""}.`
                      : `Approve ${job.target} only if the safety code it shows matches the one this device computes.`
                  }
                />
                <Button label="Approve" onPress={() => void answer(true)} fullWidth />
                <Button label="Deny" variant="secondary" onPress={() => void answer(false)} fullWidth />
              </View>
            ) : null}
            {job.phase === "failed" && job.error ? <Banner tone="danger" message={job.error} /> : null}
            {job.phase === "done" ? (
              <Banner
                tone="neutral"
                message={`${job.host?.name ?? job.target} is running${job.host?.paired ? " and paired. It appears in the Fleet within a minute." : "."}`}
              />
            ) : null}
            {isTerminalPhase(job.phase) ? (
              <Button label={job.phase === "done" ? "Done" : "Try again"} onPress={job.phase === "done" ? onClose : reset} fullWidth />
            ) : null}
          </View>
        )}
        {error ? <Banner tone="danger" message={error} /> : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: { padding: space.space16, gap: space.space16 },
  group: { gap: space.space8 },
  log: { maxHeight: 220, borderWidth: 1, borderRadius: 4, padding: space.space8 },
});
