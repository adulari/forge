// Web has no camera-scanning story (ARCHITECTURE.md §2 platform escape hatch
// "QR pairing": web pairing is paste/link only). This renders the "scan on
// your phone" hint in the same slot the native camera preview occupies, so
// `connect.tsx` mounts one `<QRScan>` regardless of platform.
import { ScanLine } from "lucide-react-native";
import React from "react";
import { StyleSheet, Text, View } from "react-native";

import { useTokens } from "../../theme/ThemeProvider";
import { radii, space } from "../../theme/tokens";
import { type as typeScale } from "../../theme/typography";

export interface QRScanProps {
  onScanned: (data: string) => void;
  paused?: boolean;
}

export function QRScan(_props: QRScanProps) {
  const tokens = useTokens();
  return (
    <View
      style={[styles.frame, { borderColor: tokens.border, backgroundColor: tokens.bg3, borderRadius: radii.radius16 }]}
      accessibilityRole="text"
      accessibilityLabel="Scan on your phone. Open Forge on your phone to scan the connect QR code, or paste the connect URL below."
    >
      <ScanLine size={24} strokeWidth={1.75} color={tokens.ink3} />
      <Text style={[typeScale.sub, styles.hint, { color: tokens.ink2 }]}>
        scan on your phone — open forge on your phone and scan the connect qr code, or paste the
        connect url below.
      </Text>
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    minHeight: 160,
    alignItems: "center",
    justifyContent: "center",
    borderWidth: StyleSheet.hairlineWidth,
    padding: space.space24,
    gap: space.space12,
  },
  hint: { textAlign: "center", maxWidth: 300 },
});
