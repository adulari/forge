// Top-level crash guard. Mounted in root `_layout.tsx` directly inside SafeAreaProvider,
// above ThemeProvider and everything else — so it must survive a THROW IN ThemeProvider
// ITSELF. That means the fallback UI can't call useTokens() or lean on any app provider:
// no theme hooks here, just plain RN primitives with hardcoded colors (see CrashScreen).
//
// Three things take the app down to the crash screen, each labelled so a report says which:
// a render error (React boundary), an uncaught error or unhandled rejection BEFORE the app first
// became interactive (a startup failure), and a native renderer that crashed in a loop. After
// startup an uncaught async error is only remembered — most are a dropped request, and blanking a
// working window for one would be worse than the error.
import * as SplashScreen from "expo-splash-screen";
import React from "react";
import { Platform } from "react-native";

import { markAnonymousTelemetryAppError } from "../lib/anonymousTelemetry";
import {
  classifyCrash,
  errorMessage,
  errorStack,
  nativeCrashLoopFromSearch,
  type CrashInfo,
  type CrashSource,
} from "../lib/crashCore";
import { reportBoundaryError } from "../lib/reporting";
import { isStartupComplete } from "../lib/startupState";
import { CrashScreen } from "./CrashScreen";

interface ErrorBoundaryProps {
  children: React.ReactNode;
}

interface ErrorBoundaryState {
  crash: CrashInfo | null;
}

function initialCrash(): CrashInfo | null {
  if (Platform.OS !== "web" || typeof window === "undefined") return null;
  if (!nativeCrashLoopFromSearch(window.location.search)) return null;
  return {
    kind: classifyCrash("native-renderer-loop", false),
    message: "The desktop shell stopped reloading the renderer after repeated crashes.",
    stack: null,
  };
}

export class ErrorBoundary extends React.Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { crash: initialCrash() };

  /** Latest uncaught error after startup, folded into the next crash report. */
  private lastBackgroundError: string | null = null;

  static getDerivedStateFromError(error: unknown): ErrorBoundaryState {
    return {
      crash: {
        kind: classifyCrash("render", isStartupComplete()),
        message: errorMessage(error),
        stack: errorStack(error),
      },
    };
  }

  componentDidMount(): void {
    if (Platform.OS !== "web" || typeof window === "undefined") return;
    window.addEventListener("error", this.onWindowError);
    window.addEventListener("unhandledrejection", this.onUnhandledRejection);
    if (this.state.crash) this.hideSplash();
  }

  componentWillUnmount(): void {
    if (Platform.OS !== "web" || typeof window === "undefined") return;
    window.removeEventListener("error", this.onWindowError);
    window.removeEventListener("unhandledrejection", this.onUnhandledRejection);
  }

  componentDidCatch(error: Error): void {
    markAnonymousTelemetryAppError("react_render");
    reportBoundaryError(error);
    console.error("[ErrorBoundary] caught render error:", error);
    // A crash before RootNavigator's normal `isLoading` effect runs would otherwise leave
    // the native splash screen up forever, hiding this fallback behind it.
    this.hideSplash();
  }

  private hideSplash(): void {
    SplashScreen.hideAsync().catch(() => {
      // best-effort — nothing sensible to do if the splash is already gone
    });
  }

  private onWindowError = (event: ErrorEvent): void => {
    // Resource load failures (a broken <img>) arrive here with no error object.
    if (!event.error && !event.message) return;
    this.handleAsync("script-error", event.error ?? event.message);
  };

  private onUnhandledRejection = (event: PromiseRejectionEvent): void => {
    this.handleAsync("unhandled-rejection", event.reason);
  };

  private handleAsync(source: Extract<CrashSource, "script-error" | "unhandled-rejection">, value: unknown): void {
    const message = errorMessage(value);
    if (isStartupComplete()) {
      this.lastBackgroundError = `${source}: ${message}`;
      return;
    }
    if (this.state.crash) return;
    this.setState({
      crash: { kind: classifyCrash(source, false), message, stack: errorStack(value) },
    });
    this.hideSplash();
  }

  reset = (): void => {
    this.setState({ crash: null });
  };

  render(): React.ReactNode {
    const { crash } = this.state;
    if (crash) {
      const info =
        this.lastBackgroundError && crash.kind === "render"
          ? { ...crash, message: `${crash.message} (after ${this.lastBackgroundError})` }
          : crash;
      return <CrashScreen info={info} onTryAgain={crash.kind === "renderer-loop" ? undefined : this.reset} />;
    }
    return this.props.children;
  }
}
