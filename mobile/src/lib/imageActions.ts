// Copy and save for an image the viewer shows, on iOS and Android. The picture is downloaded to
// the cache first; "save" hands it to the share sheet, which offers Save Image / Save to Files.
import * as Clipboard from "expo-clipboard";
import { Directory, File, Paths } from "expo-file-system";
import * as Sharing from "expo-sharing";

import type { ViewerImage } from "./imageTypes";
import { downloadName } from "./imageTypes";

async function download(image: ViewerImage): Promise<File> {
  const folder = new Directory(Paths.cache, "forge-images");
  if (!folder.exists) folder.create();
  return File.downloadFileAsync(image.uri, new File(folder, downloadName(image)), { idempotent: true });
}

export async function copyImage(image: ViewerImage): Promise<void> {
  const file = await download(image);
  await Clipboard.setImageAsync(await file.base64());
}

export async function saveImage(image: ViewerImage): Promise<string> {
  const file = await download(image);
  if (!(await Sharing.isAvailableAsync())) throw new Error("Saving images is not available on this device.");
  await Sharing.shareAsync(file.uri, { dialogTitle: "Save image" });
  return "Opened the share sheet";
}
