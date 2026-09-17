// Platform-appropriate wording for the biometric app-lock toggle/gate (Settings + AppLock.tsx).
// Android has no single OS-branded biometric name — the system's own prompt already names
// whatever it used (fingerprint/face unlock), so "biometric unlock" is the honest generic term
// there. iOS genuinely has two distinct, Apple-branded names, so it's worth asking
// expo-local-authentication which one the device actually reports rather than hardcoding
// "Face ID" for a Touch ID device.
import { Platform } from "react-native";
import * as LocalAuthentication from "expo-local-authentication";

export const ANDROID_BIOMETRIC_LABEL = "biometric unlock";

let cached: string | null = null;

/** Resolves to "Face ID", "Touch ID", or "biometric unlock" depending on platform and (on iOS)
 * what hardware the device reports. Cached — the answer can't change while the app runs. */
export async function biometricLabel(): Promise<string> {
  if (Platform.OS !== "ios") return ANDROID_BIOMETRIC_LABEL;
  if (cached) return cached;
  try {
    const types = await LocalAuthentication.supportedAuthenticationTypesAsync();
    cached = types.includes(LocalAuthentication.AuthenticationType.FACIAL_RECOGNITION)
      ? "Face ID"
      : types.includes(LocalAuthentication.AuthenticationType.FINGERPRINT)
        ? "Touch ID"
        : "Face ID";
  } catch {
    cached = "Face ID";
  }
  return cached;
}

/** Synchronous best-guess for first paint, before `biometricLabel()` resolves — Android needs
 * no async call at all, and iOS defaults to "Face ID" (the common case) until the real answer
 * is in. */
export function biometricLabelDefault(): string {
  return Platform.OS === "ios" ? "Face ID" : ANDROID_BIOMETRIC_LABEL;
}
