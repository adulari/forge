// Top-level crash guard. Mounted in root `_layout.tsx` directly inside SafeAreaProvider,
// above ThemeProvider and everything else — so it must survive a THROW IN ThemeProvider
// ITSELF. That means the fallback UI can't call useTokens() or lean on any app provider:
// no theme hooks here, just plain RN primitives with hardcoded colors.
import * as SplashScreen from "expo-splash-screen";
import React from "react";
import { Platform, Pressable, StyleSheet, Text, View } from "react-native";

import { markAnonymousTelemetryAppError } from "../lib/anonymousTelemetry";

interface ErrorBoundaryProps {
  children: React.ReactNode;
}

interface ErrorBoundaryState {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends React.Component<ErrorBoundaryProps, ErrorBoundaryState> {
  state: ErrorBoundaryState = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): ErrorBoundaryState {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error): void {
    markAnonymousTelemetryAppError("react_render");
    console.error("[ErrorBoundary] caught render error:", error);
    // A crash before RootNavigator's normal `isLoading` effect runs would otherwise leave
    // the native splash screen up forever, hiding this fallback behind it.
    SplashScreen.hideAsync().catch(() => {
      // best-effort — nothing sensible to do if the splash is already gone
    });
  }

  reset = (): void => {
    this.setState({ hasError: false, error: null });
  };

  render(): React.ReactNode {
    if (this.state.hasError) {
      return (
        <View style={styles.container}>
          <Text style={styles.title}>something went wrong</Text>
          <Text style={styles.message}>{this.state.error?.message ?? "unknown error"}</Text>
          <Pressable
            style={styles.button}
            onPress={this.reset}
            accessibilityRole="button"
            accessibilityLabel="Try loading Forge again"
          >
            <Text style={styles.buttonText}>Try again</Text>
          </Pressable>
        </View>
      );
    }
    return this.props.children;
  }
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor: "#111111",
    paddingHorizontal: 24,
  },
  title: {
    color: "#dddddd",
    fontSize: 18,
    fontWeight: "600",
    marginBottom: 12,
  },
  message: {
    color: "#999999",
    fontSize: 13,
    // Not monoFamily — JetBrains Mono may not have loaded when this renders, and
    // iOS has no "monospace" alias.
    fontFamily: Platform.select({ ios: "Menlo", default: "monospace" }),
    textAlign: "center",
    marginBottom: 24,
  },
  button: {
    minHeight: 44,
    borderWidth: 1,
    borderColor: "#dddddd",
    borderRadius: 8,
    paddingHorizontal: 20,
    paddingVertical: 10,
    justifyContent: "center",
  },
  buttonText: {
    color: "#dddddd",
    fontSize: 14,
    fontWeight: "500",
  },
});
