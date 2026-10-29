import { router, usePathname } from "expo-router";
import React, { useMemo } from "react";
import { Platform } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";

import { useAppVersion } from "../lib/appVersion";
import { assessCompatibility, mismatchMessage } from "../lib/diagnostics";
import { useDiagnostics } from "../lib/queries";
import { PROTOCOL_VERSION } from "../lib/remoteProtocol";
import { UpdateBannerHost } from "./UpdateBannerHost";
import { Banner } from "./ds/Banner";

function OperationalBanner(props: React.ComponentProps<typeof Banner>) {
  return (
    <SafeAreaView edges={Platform.OS === "ios" ? ["top"] : []}>
      <Banner {...props} />
    </SafeAreaView>
  );
}

export function OperationalNotice() {
  const pathname = usePathname();
  const diagnostics = useDiagnostics();
  const appVersion = useAppVersion();
  const compatibility = useMemo(
    () => assessCompatibility(
      diagnostics.data?.host.protocol,
      diagnostics.data?.host.version,
      PROTOCOL_VERSION,
      appVersion,
    ),
    [appVersion, diagnostics.data?.host.protocol, diagnostics.data?.host.version],
  );

  if (pathname.startsWith("/diagnostics") || pathname.startsWith("/session/")) return null;

  if (compatibility.status === "daemon-outdated") {
    return (
      <OperationalBanner
        compact
        tone="warn"
        message={mismatchMessage("daemon-outdated", diagnostics.data?.host.protocol, PROTOCOL_VERSION)}
        actionLabel="Fix"
        onAction={() => router.push("/diagnostics")}
      />
    );
  }
  if (compatibility.status === "client-outdated") {
    return (
      <OperationalBanner
        compact
        tone="warn"
        message={mismatchMessage("client-outdated", diagnostics.data?.host.protocol, PROTOCOL_VERSION)}
        actionLabel="Update"
        onAction={() => router.push("/diagnostics")}
      />
    );
  }
  if (compatibility.status === "client-limited") {
    return (
      <OperationalBanner
        compact
        tone="warn"
        message={compatibility.detail}
        actionLabel="Details"
        onAction={() => router.push("/diagnostics")}
      />
    );
  }
  return <UpdateBannerHost appVersion={appVersion} />;
}
