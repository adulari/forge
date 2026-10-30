import * as Clipboard from "expo-clipboard";
import { ClipboardPaste, ImageUp } from "lucide-react-native";
import React, { useEffect, useState } from "react";
import { StyleSheet, Text, View } from "react-native";

import { composeManualLink } from "../../lib/connectUrl";
import { decodeQrFromFile, qrImageSupported } from "../../lib/qrImage";
import { space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";
import { useTokens } from "../../theme/ThemeProvider";
import { Button } from "../ds/Button";
import { Input } from "../ds/Input";
import { Sheet } from "../ds/Sheet";

export interface ManualPrefill {
  host: string;
  port: string;
  fingerprint?: string;
}

export interface OtherWaysSheetProps {
  visible: boolean;
  onClose: () => void;
  onSubmit: (link: string) => void;
  prefill?: ManualPrefill | null;
}

export function OtherWaysSheet({ visible, onClose, onSubmit, prefill }: OtherWaysSheetProps) {
  const tokens = useTokens();
  const [pasted, setPasted] = useState("");
  const [host, setHost] = useState("");
  const [port, setPort] = useState("7420");
  const [token, setToken] = useState("");
  const [photoNote, setPhotoNote] = useState<string | null>(null);

  useEffect(() => {
    if (!visible || !prefill) return;
    setHost(prefill.host);
    setPort(prefill.port);
  }, [visible, prefill]);

  const pasteFromClipboard = async () => {
    try {
      const text = await Clipboard.getStringAsync();
      if (text) setPasted(text.trim());
    } catch {
      setPhotoNote("couldn't read the clipboard — paste into the field instead.");
    }
  };

  const pickPhoto = async () => {
    setPhotoNote(null);
    try {
      const value = await decodeQrFromFile();
      if (value) onSubmit(value);
      else setPhotoNote("no qr code found in that image.");
    } catch {
      setPhotoNote("couldn't read that image.");
    }
  };

  const manualLink = composeManualLink(host, port, token, prefill?.fingerprint);

  return (
    <Sheet visible={visible} onClose={onClose} accessibilityLabel="Other ways to connect" maxHeightRatio={0.92}>
      <View style={styles.body}>
        <Text style={[typeScale.title, { color: tokens.ink }]}>Other ways to connect</Text>

        <View style={styles.group}>
          <Text style={[typeScale.section, { color: tokens.ink4 }]}>paste a link</Text>
          <Input
            mono
            value={pasted}
            onChangeText={(t) => setPasted(t.trimStart())}
            placeholder="https://host:7420/#token=…"
            autoCapitalize="none"
            autoCorrect={false}
            returnKeyType="go"
            onSubmitEditing={() => pasted.trim() && onSubmit(pasted)}
            accessibilityLabel="Connect URL"
          />
          <Button
            label="Paste from clipboard"
            variant="secondary"
            icon={<ClipboardPaste size={16} strokeWidth={1.75} color={tokens.ink} />}
            onPress={() => void pasteFromClipboard()}
            fullWidth
          />
          <Button label="Continue" onPress={() => onSubmit(pasted)} disabled={pasted.trim().length === 0} fullWidth />
        </View>

        {qrImageSupported ? (
          <View style={styles.group}>
            <Text style={[typeScale.section, { color: tokens.ink4 }]}>photo of the qr code</Text>
            <Button
              label="Choose a photo"
              variant="secondary"
              icon={<ImageUp size={16} strokeWidth={1.75} color={tokens.ink} />}
              onPress={() => void pickPhoto()}
              fullWidth
            />
          </View>
        ) : null}
        {photoNote ? <Text style={[typeScale.sub, { color: tokens.ink3 }]}>{photoNote}</Text> : null}

        <View style={styles.group}>
          <Text style={[typeScale.section, { color: tokens.ink4 }]}>enter it by hand</Text>
          <Input label="Host" mono value={host} onChangeText={setHost} placeholder="192.168.1.20" autoCapitalize="none" autoCorrect={false} />
          <Input label="Port" mono value={port} onChangeText={setPort} placeholder="7420" keyboardType="number-pad" />
          <Input label="Token" mono value={token} onChangeText={setToken} placeholder="from `forge serve pair`" autoCapitalize="none" autoCorrect={false} />
          {prefill?.fingerprint ? (
            <Text style={[typeScale.monoMeta, { color: tokens.ink4 }]} numberOfLines={1}>
              pinned to the certificate this host advertised
            </Text>
          ) : null}
          <Button label="Continue" onPress={() => manualLink && onSubmit(manualLink)} disabled={!manualLink} fullWidth />
        </View>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create({
  body: { padding: space.space16, gap: space.space20 },
  group: { gap: space.space8 },
});
