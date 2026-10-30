import { Share } from "react-native";

import { shareCardLines, type ShareCardInput } from "./usageShareCardCore";

export const canSaveShareImage = false;

export async function shareUsageCard(input: ShareCardInput): Promise<"shared" | "failed"> {
  try {
    await Share.share({ message: shareCardLines(input).join("\n") });
    return "shared";
  } catch {
    return "failed";
  }
}
