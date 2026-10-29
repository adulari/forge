// Webview side of the shell's launch-at-login settings (src-tauri/src/desktop_shell.rs).
import { isTauri } from "./platform";

export interface LaunchSettings {
  launchAtLogin: boolean;
  startMinimized: boolean;
}

async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const { invoke: call } = await import("@tauri-apps/api/core");
  return call<T>(command, args);
}

export async function getLaunchSettings(): Promise<LaunchSettings | null> {
  if (!isTauri) return null;
  try {
    return await invoke<LaunchSettings>("launch_settings");
  } catch {
    return null;
  }
}

export function setLaunchAtLogin(enabled: boolean): Promise<void> {
  return invoke("set_launch_at_login", { enabled });
}

export function setStartMinimized(enabled: boolean): Promise<void> {
  return invoke("set_start_minimized", { enabled });
}
