import { useEffect, useRef, useState } from "react";
import { Platform, ScrollView, StyleSheet, Text, View } from "react-native";

import {
  NEVER_SENT,
  approvePending,
  buildManualReport,
  dismissPending,
  installReporting,
  lastErrorPreview,
  sendManualReport,
  useReporting,
  type Report,
} from "../lib/reporting";
import { useTokens } from "../theme/ThemeProvider";
import { radii, space } from "../theme/tokens";
import { monoFamily, type } from "../theme/typography";
import { Button } from "./ds/Button";
import { Checkbox } from "./ds/Checkbox";
import { Input } from "./ds/Input";
import { Sheet } from "./ds/Sheet";
import { useToast } from "./ds/ToastHost";

function ReportPreview({ report }: { report: Report }) {
  const tokens = useTokens();
  return (
    <ScrollView
      style={[styles.preview, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}
      nestedScrollEnabled
    >
      <Text selectable style={[styles.mono, { color: tokens.ink2 }]}>
        {JSON.stringify(report, null, 2)}
      </Text>
    </ScrollView>
  );
}

/** Ask mode: one card per report, with the exact payload and three choices. */
function ReportCard({ report }: { report: Report }) {
  const tokens = useTokens();
  const [showing, setShowing] = useState(false);
  return (
    <View
      testID="report-card"
      accessibilityLabel="Forge hit a problem. Send an anonymous report?"
      style={[styles.card, { backgroundColor: tokens.bg2, borderColor: tokens.border }]}
    >
      <Text style={[type.bodyBold, { color: tokens.ink }]}>Forge hit a problem</Text>
      <Text style={[type.sub, { color: tokens.ink3 }]}>
        Send an anonymous report so it can be fixed? It has an error type, a short scrubbed message and
        app version, and no prompts, code, paths or keys.
      </Text>
      <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={2}>
        {report.error_kind} {report.location}
      </Text>
      {showing ? <ReportPreview report={report} /> : null}
      <View style={styles.actions}>
        <Button label="Send" onPress={() => void approvePending(report.fingerprint)} />
        <Button label="Don't send" variant="secondary" onPress={() => dismissPending(report.fingerprint)} />
        <Button label="Always send" variant="secondary" onPress={() => void approvePending(report.fingerprint, true)} />
        <Button
          label={showing ? "Hide preview" : "Preview"}
          variant="ghost"
          onPress={() => setShowing((v) => !v)}
        />
      </View>
    </View>
  );
}

/**
 * Mounted once under the root providers: installs the global handlers, shows the ask-mode card,
 * and in auto mode a single quiet toast when a report went out.
 */
export function ReportingHost() {
  const toast = useToast();
  const { pending, lastAutoSentAt } = useReporting();
  const markedAt = useRef<number | null>(null);

  useEffect(() => installReporting(), []);

  useEffect(() => {
    if (lastAutoSentAt && lastAutoSentAt !== markedAt.current) {
      markedAt.current = lastAutoSentAt;
      toast.show("Problem reported anonymously. Details are in Settings under Error reports.", {
        duration: 4_000,
      });
    }
  }, [lastAutoSentAt, toast]);

  const first = pending[0];
  if (!first) return null;
  return (
    <View pointerEvents="box-none" style={styles.host}>
      <ReportCard report={first} />
    </View>
  );
}

/** "What gets sent": the latest report verbatim, what is collected and what never is. */
export function WhatGetsSentSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const tokens = useTokens();
  const { sent } = useReporting();
  const last = sent[0]?.report;
  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="What gets sent">
      <ScrollView contentContainerStyle={styles.sheetBody}>
        <Text style={[type.bodyBold, { color: tokens.ink }]}>What gets sent</Text>
        <Text style={[type.sub, { color: tokens.ink3 }]}>
          When Forge hits a bug it sends the error type, a short scrubbed message, the code location, the app
          version, your OS and a random install id, plus the kinds and durations of the last few app events.
          Home folders become ~, other paths shrink to a file name, URLs shrink to a host.
        </Text>
        <Text style={[type.bodyBold, { color: tokens.ink }]}>Never sent</Text>
        {NEVER_SENT.map((line) => (
          <Text key={line} style={[type.sub, { color: tokens.ink3 }]}>
            {`- ${line}`}
          </Text>
        ))}
        <Text style={[type.bodyBold, { color: tokens.ink }]}>Last report sent from this device</Text>
        {last ? (
          <ReportPreview report={last} />
        ) : (
          <Text style={[type.sub, { color: tokens.ink3 }]}>Nothing has been sent from this device.</Text>
        )}
        <Button label="Close" variant="secondary" onPress={onClose} />
      </ScrollView>
    </Sheet>
  );
}

/** Manual path: the user's own description plus an optional, previewed last-error attachment. */
export function ReportProblemSheet({ visible, onClose }: { visible: boolean; onClose: () => void }) {
  const tokens = useTokens();
  const toast = useToast();
  const [note, setNote] = useState("");
  const [attach, setAttach] = useState(false);
  const [preview, setPreview] = useState<Report | null>(null);
  const [busy, setBusy] = useState(false);
  const lastError = lastErrorPreview();

  useEffect(() => {
    if (!visible) {
      setNote("");
      setAttach(false);
      setPreview(null);
    }
  }, [visible]);

  const review = async () => {
    setPreview(await buildManualReport(note, attach && lastError ? lastError : undefined));
  };
  const send = async () => {
    if (!preview) return;
    setBusy(true);
    const ok = await sendManualReport(preview);
    setBusy(false);
    toast.show(ok ? "Thanks. Your report was sent." : "Couldn't send the report. Check that error reports are on.", {
      tone: ok ? undefined : "danger",
    });
    if (ok) onClose();
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Report a problem">
      <ScrollView contentContainerStyle={styles.sheetBody} keyboardShouldPersistTaps="handled">
        <Text style={[type.bodyBold, { color: tokens.ink }]}>Report a problem</Text>
        {preview ? (
          <>
            <Text style={[type.sub, { color: tokens.ink3 }]}>This is exactly what will be sent.</Text>
            <ReportPreview report={preview} />
            <View style={styles.actions}>
              <Button label="Send report" loading={busy} onPress={() => void send()} />
              <Button label="Back" variant="secondary" onPress={() => setPreview(null)} />
            </View>
          </>
        ) : (
          <>
            <Text style={[type.sub, { color: tokens.ink3 }]}>
              Describe what went wrong. Please leave out passwords and private details; the report also
              carries the same anonymous app context as automatic reports.
            </Text>
            <Input
              label="What happened?"
              value={note}
              onChangeText={setNote}
              multiline
              numberOfLines={4}
              testID="report-note"
            />
            {lastError ? (
              <View style={styles.attachRow}>
                <Checkbox value={attach} onValueChange={setAttach} accessibilityLabel="Attach the last error text" />
                <View style={styles.flex}>
                  <Text style={[type.body, { color: tokens.ink }]}>Attach the last error text</Text>
                  <Text style={[type.monoMeta, { color: tokens.ink4 }]} numberOfLines={3}>
                    {lastError}
                  </Text>
                </View>
              </View>
            ) : null}
            <View style={styles.actions}>
              <Button label="Review" disabled={note.trim().length < 3} onPress={() => void review()} />
              <Button label="Cancel" variant="secondary" onPress={onClose} />
            </View>
          </>
        )}
      </ScrollView>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  host: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: space.space16,
    alignItems: "center",
    zIndex: 50,
  },
  card: {
    width: "94%",
    maxWidth: 520,
    borderWidth: StyleSheet.hairlineWidth,
    borderRadius: radii.radius8,
    padding: space.space12,
    gap: space.space8,
  },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: space.space8 },
  preview: { maxHeight: 220, borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius8, padding: space.space8 },
  mono: { fontFamily: Platform.select({ ios: "Menlo", default: "monospace" }) ?? monoFamily.regular, fontSize: 11 },
  sheetBody: { padding: space.space16, gap: space.space12 },
  attachRow: { flexDirection: "row", gap: space.space12, alignItems: "flex-start" },
  flex: { flex: 1 },
});
