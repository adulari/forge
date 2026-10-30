interface BarcodeDetectorLike {
  detect(source: ImageBitmapSource): Promise<{ rawValue: string }[]>;
}
type BarcodeDetectorCtor = new (options: { formats: string[] }) => BarcodeDetectorLike;

function detectorCtor(): BarcodeDetectorCtor | null {
  if (typeof window === "undefined") return null;
  return (window as unknown as { BarcodeDetector?: BarcodeDetectorCtor }).BarcodeDetector ?? null;
}

export const qrImageSupported = detectorCtor() !== null && typeof document !== "undefined";

/** Ask the browser for an image file and read the QR code in it. Null when none is found. */
export async function decodeQrFromFile(): Promise<string | null> {
  const Detector = detectorCtor();
  if (!Detector || typeof document === "undefined") return null;
  const file = await new Promise<File | null>((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = "image/*";
    input.onchange = () => resolve(input.files?.[0] ?? null);
    input.oncancel = () => resolve(null);
    input.click();
  });
  if (!file) return null;
  const bitmap = await createImageBitmap(file);
  try {
    const found = await new Detector({ formats: ["qr_code"] }).detect(bitmap);
    return found[0]?.rawValue ?? null;
  } finally {
    bitmap.close();
  }
}
