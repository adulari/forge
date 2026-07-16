import { isTauri } from "./platform";
import { isLoopbackServer } from "./projectSelection";

export function canChooseNativeFolder(baseUrl: string | null): boolean {
  return isTauri && isLoopbackServer(baseUrl);
}

export async function chooseNativeFolder(): Promise<string | null> {
  if (!isTauri) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const selected = await open({
    directory: true,
    multiple: false,
    title: "Choose a Forge project",
  });
  return typeof selected === "string" ? selected : null;
}
