// Copy and save for an image the viewer shows, in a browser or the desktop webview.
import type { ViewerImage } from "./imageTypes";
import { downloadName } from "./imageTypes";
import { tildify } from "./pathDisplay";
import { isTauri } from "./platform";

async function fetchBlob(image: ViewerImage): Promise<Blob> {
  const response = await fetch(image.uri);
  if (!response.ok) throw new Error(`Could not load the image (${response.status}).`);
  return response.blob();
}

async function asPng(blob: Blob): Promise<Blob> {
  if (blob.type === "image/png") return blob;
  const bitmap = await createImageBitmap(blob);
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.getContext("2d")?.drawImage(bitmap, 0, 0);
  bitmap.close();
  return new Promise((resolve, reject) =>
    canvas.toBlob((png) => (png ? resolve(png) : reject(new Error("Could not convert the image."))), "image/png"),
  );
}

export async function copyImage(image: ViewerImage): Promise<void> {
  if (typeof ClipboardItem === "undefined" || !navigator.clipboard?.write) {
    throw new Error("This browser cannot copy images here. Use Save instead.");
  }
  // The clipboard write has to start inside the click that asked for it (WebKit refuses it after
  // an await), so it gets the image as a promise rather than waiting for the download first.
  const png = fetchBlob(image).then(asPng);
  try {
    await navigator.clipboard.write([new ClipboardItem({ "image/png": png })]);
  } catch (error) {
    if (error instanceof DOMException && error.name === "NotAllowedError") {
      throw new Error("Copying was blocked here. Use Save instead.");
    }
    throw error;
  }
}

async function saveWithShell(blob: Blob, name: string): Promise<string> {
  const { invoke } = await import("@tauri-apps/api/core");
  const path = await invoke<string>("save_image", new Uint8Array(await blob.arrayBuffer()), {
    headers: { "x-file-name": encodeURIComponent(name) },
  });
  return `Saved to ${tildify(path)}`;
}

export async function saveImage(image: ViewerImage): Promise<string> {
  const blob = await fetchBlob(image);
  if (isTauri) return saveWithShell(blob, downloadName(image, blob.type));
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = downloadName(image, blob.type);
  document.body.appendChild(link);
  link.click();
  link.remove();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return "Saved to your downloads";
}
