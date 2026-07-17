// New pattern 9 (Structured-output block): a bg0 card of pretty-printed JSON with
// color-coded tokens — info keys, success strings, warn-ink numbers/booleans — and a copy
// affordance in the header. Reused by the agent drill-in and the workflow result summary.
import * as Clipboard from "expo-clipboard";
import { Check, Copy } from "lucide-react-native";
import React, { useMemo, useState } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";

import { useToast } from "../ds/ToastHost";
import { useTokens } from "../../theme/ThemeProvider";
import type { ColorTokens } from "../../theme/tokens";
import { radii, space } from "../../theme/tokens";
import { monoFamily, type as typeScale } from "../../theme/typography";

// One pass over the pretty string: a quoted string (optionally a key when a colon trails),
// a number, or a boolean/null literal. Everything between matches (punctuation, braces,
// whitespace, newlines) is emitted verbatim in the default ink so wrapping is preserved.
const JSON_TOKEN = /("(?:[^"\\]|\\.)*")(\s*:)?|(-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)|\b(true|false|null)\b/g;

function highlight(pretty: string, tokens: ColorTokens): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  let last = 0;
  let key = 0;
  let match: RegExpExecArray | null;
  JSON_TOKEN.lastIndex = 0;
  while ((match = JSON_TOKEN.exec(pretty)) !== null) {
    if (match.index > last) {
      nodes.push(
        <Text key={key++} style={{ color: tokens.ink2 }}>
          {pretty.slice(last, match.index)}
        </Text>,
      );
    }
    const [full, str, colon, num, bool] = match;
    if (str !== undefined) {
      if (colon !== undefined && colon !== "") {
        nodes.push(
          <Text key={key++} style={{ color: tokens.info }}>
            {str}
          </Text>,
        );
        nodes.push(
          <Text key={key++} style={{ color: tokens.ink2 }}>
            {colon}
          </Text>,
        );
      } else {
        nodes.push(
          <Text key={key++} style={{ color: tokens.success }}>
            {str}
          </Text>,
        );
      }
    } else if (num !== undefined) {
      nodes.push(
        <Text key={key++} style={{ color: tokens.warnBgInk }}>
          {num}
        </Text>,
      );
    } else if (bool !== undefined) {
      nodes.push(
        <Text key={key++} style={{ color: tokens.warnBgInk }}>
          {bool}
        </Text>,
      );
    }
    last = match.index + full.length;
  }
  if (last < pretty.length) {
    nodes.push(
      <Text key={key++} style={{ color: tokens.ink2 }}>
        {pretty.slice(last)}
      </Text>,
    );
  }
  return nodes;
}

export function StructuredOutput({ data, label }: { data: unknown; label: string }) {
  const tokens = useTokens();
  const toast = useToast();
  const [copied, setCopied] = useState(false);

  const pretty = useMemo(() => {
    try {
      return JSON.stringify(data, null, 2);
    } catch {
      return String(data);
    }
  }, [data]);
  const nodes = useMemo(() => highlight(pretty, tokens), [pretty, tokens]);

  const onCopy = () => {
    Clipboard.setStringAsync(pretty)
      .then(() => {
        setCopied(true);
        toast.show("copied");
        setTimeout(() => setCopied(false), 1500);
      })
      .catch(() => toast.show("couldn't copy", { tone: "danger" }));
  };

  return (
    <View style={[styles.card, { backgroundColor: tokens.bg0, borderColor: tokens.border }]}>
      <View style={[styles.header, { borderBottomColor: tokens.border }]}>
        <Text style={[styles.label, { color: tokens.ink3 }]} numberOfLines={1}>
          {label}
        </Text>
        <Pressable
          onPress={onCopy}
          accessibilityRole="button"
          accessibilityLabel="Copy JSON"
          hitSlop={8}
          style={styles.copy}
        >
          {copied ? (
            <Check size={14} strokeWidth={1.75} color={tokens.success} />
          ) : (
            <Copy size={14} strokeWidth={1.75} color={tokens.ink3} />
          )}
        </Pressable>
      </View>
      <Text style={[styles.body, { color: tokens.ink2 }]} selectable>
        {nodes}
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  card: { borderWidth: StyleSheet.hairlineWidth, borderRadius: radii.radius12, overflow: "hidden" },
  header: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: space.space12,
    paddingVertical: 6,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  label: { ...typeScale.monoMeta, fontSize: 10.5, flexShrink: 1 },
  copy: { minWidth: 24, minHeight: 24, alignItems: "center", justifyContent: "center" },
  body: {
    fontFamily: monoFamily.regular,
    fontSize: 12,
    lineHeight: 19,
    paddingHorizontal: space.space12,
    paddingVertical: 10,
  },
});
