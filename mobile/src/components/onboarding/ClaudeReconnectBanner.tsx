// Outside setup: Claude Code is installed and signed in, but Forge no longer shares that login
// with its bridge, so it needs its own token. One click runs the same connection job as setup.
import { router, usePathname } from "expo-router";
import React, { useState } from "react";
import { View } from "react-native";

import { Banner } from "../ds/Banner";
import { useTokens } from "../../theme/ThemeProvider";
import { space } from "../../theme/tokens";
import { useOnboardingBrief } from "./brief";

export function ClaudeReconnectBanner() {
  const tokens = useTokens();
  const brief = useOnboardingBrief();
  const pathname = usePathname();
  const [hidden, setHidden] = useState(false);
  if (!brief?.claude_reconnect || hidden || /^\/onboarding(\/|$)/.test(pathname)) return null;
  return (
    <View
      style={{ paddingHorizontal: space.space12, paddingVertical: space.space8, backgroundColor: tokens.bg0 }}
      testID="claude-reconnect-banner"
    >
      <Banner
        tone="warn"
        message="Claude Code needs to be reconnected. It takes one sign-in."
        actionLabel="Reconnect Claude Code"
        onAction={() => router.push("/onboarding?focus=claude")}
        onDismiss={() => setHidden(true)}
      />
    </View>
  );
}
