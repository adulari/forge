import { shareCardLines, type ShareCardInput } from "./usageShareCardCore";

export const canSaveShareImage = true;

const WIDTH = 1200;
const HEIGHT = 630;

export function paintShareCard(ctx: CanvasRenderingContext2D, input: ShareCardInput): void {
  const { palette, weeks } = input;
  ctx.fillStyle = palette.bg;
  ctx.fillRect(0, 0, WIDTH, HEIGHT);
  const lines = shareCardLines(input);
  ctx.textBaseline = "top";
  ctx.fillStyle = palette.accent;
  ctx.font = "600 34px ui-monospace, Menlo, monospace";
  ctx.fillText(lines[0], 60, 50);
  ctx.fillStyle = palette.ink;
  ctx.font = "700 50px ui-monospace, Menlo, monospace";
  ctx.fillText(lines[1], 60, 110);
  ctx.fillStyle = palette.ink3;
  ctx.font = "400 30px ui-monospace, Menlo, monospace";
  ctx.fillText(lines[2], 60, 200);
  const cell = 17;
  const gap = 4;
  const left = 60;
  const top = 280;
  weeks.forEach((column, x) => {
    column.forEach((day, y) => {
      if (!day) return;
      ctx.fillStyle = day.level === 0 || !day.inWindow ? palette.track : palette.levels[day.level - 1];
      ctx.fillRect(left + x * (cell + gap), top + y * (cell + gap), cell, cell);
    });
  });
  ctx.fillStyle = palette.ink3;
  ctx.font = "400 24px ui-monospace, Menlo, monospace";
  ctx.fillText(lines[3], 60, HEIGHT - 60);
}

export async function shareUsageCard(input: ShareCardInput): Promise<"saved" | "failed"> {
  if (typeof document === "undefined") return "failed";
  const canvas = document.createElement("canvas");
  canvas.width = WIDTH;
  canvas.height = HEIGHT;
  const ctx = canvas.getContext("2d");
  if (!ctx) return "failed";
  paintShareCard(ctx, input);
  const blob = await new Promise<Blob | null>((resolve) => canvas.toBlob(resolve, "image/png"));
  if (!blob) return "failed";
  const url = URL.createObjectURL(blob);
  const link = document.createElement("a");
  link.href = url;
  link.download = "forge-activity.png";
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 10_000);
  return "saved";
}
