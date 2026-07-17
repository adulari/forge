import * as Clipboard from "expo-clipboard";
import { Copy, PencilLine, Quote } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { displayMessageText } from "./MessageRow";
import type { HistoryRow } from "../../lib/api";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { ListRow } from "../ds/ListRow";
import { Sheet } from "../ds/Sheet";
import { useToast } from "../ds/ToastHost";

export interface MessageActionsSheetProps {
  visible: boolean;
  message: HistoryRow | null;
  onClose: () => void;
  onQuote: (text: string) => void;
  /** Load this message's text into the composer for editing (user messages only). */
  onEdit?: (text: string) => void;
}

function firstCodeBlock(text: string): string | null {
  const match = text.match(/```[^\n]*\n([\s\S]*?)```/);
  return match ? match[1].replace(/\n$/, "") : null;
}

export function MessageActionsSheet({ visible, message, onClose, onQuote, onEdit }: MessageActionsSheetProps) {
  const tokens = useTokens();
  const toast = useToast();
  const text = message ? displayMessageText(message) : "";
  const code = firstCodeBlock(text);
  const editable = message?.role === "user" && onEdit != null;

  const copy = async (value: string) => {
    await Clipboard.setStringAsync(value);
    toast.show("Copied");
    onClose();
  };

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Message actions">
      <View style={styles.content}>
        <Text style={[typeScale.heading, { color: tokens.ink }]}>Message actions</Text>
        <ListRow
          title="Copy text"
          leading={<Copy size={18} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => void copy(text)}
          showSeparator={false}
        />
        {code !== null ? (
          <ListRow
            title="Copy code"
            leading={<Copy size={18} strokeWidth={1.75} color={tokens.ink2} />}
            onPress={() => void copy(code)}
            showSeparator={false}
          />
        ) : null}
        <ListRow
          title="Quote"
          leading={<Quote size={18} strokeWidth={1.75} color={tokens.ink2} />}
          onPress={() => {
            onQuote(text);
            onClose();
          }}
          showSeparator={false}
        />
        {editable ? (
          <ListRow
            title="Edit & resend"
            leading={<PencilLine size={18} strokeWidth={1.75} color={tokens.ink2} />}
            onPress={() => {
              onEdit!(text);
              onClose();
            }}
            showSeparator={false}
          />
        ) : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  content: { paddingBottom: space.space16, gap: space.space4 },
});
