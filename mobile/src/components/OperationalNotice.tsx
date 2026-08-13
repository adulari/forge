import { router, usePathname } from "expo-router";
import React, { useMemo } from "react";

import { useAppVersion } from "../lib/appVersion";
import { assessCompatibility } from "../lib/diagnostics";
import { useDiagnostics } from "../lib/queries";
import { PROTOCOL_VERSION } from "../lib/remoteProtocol";
import { useDesktopUpdateState } from "../lib/updater";
import { Banner } from "./ds/Banner";

export function OperationalNotice() {
  const pathname = usePathname();
  const diagnostics = useDiagnostics();
  const update = useDesktopUpdateState();
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

  if (pathname.startsWith("/diagnostics")) return null;

  if (compatibility.status === "daemon-outdated") {
    return (
      <Banner
        compact
        tone="warn"
        message={`Daemon protocol v${diagnostics.data?.host.protocol} is older than this app's v${PROTOCOL_VERSION}.`}
        actionLabel="Fix"
        onAction={() => router.push("/diagnostics")}
      />
    );
  }
  if (compatibility.status === "client-outdated") {
    return (
      <Banner
        compact
        tone="warn"
        message={`This app's protocol v${PROTOCOL_VERSION} is older than the daemon's v${diagnostics.data?.host.protocol}.`}
        actionLabel="Update"
        onAction={() => router.push("/diagnostics")}
      />
    );
  }
  if (update.phase === "available") {
    return (
      <Banner
        compact
        tone="neutral"
        message={`Available desktop update: Forge ${update.availableVersion} · Installed app: ${appVersion}`}
        actionLabel="View"
        onAction={() => router.push("/diagnostics")}
      />
    );
  }
  return null;
}
