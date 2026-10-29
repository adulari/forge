const ZOOM_STEPS = [0.25, 0.5, 0.75, 1, 1.5, 2, 3, 4, 6, 8];

/** The scale that makes an image fit its stage, never enlarging a small image past its own size. */
export function fitScale(image: { width: number; height: number }, stage: { width: number; height: number }): number {
  if (image.width <= 0 || image.height <= 0 || stage.width <= 0 || stage.height <= 0) return 1;
  const padding = 24;
  const room = { width: Math.max(stage.width - padding, 1), height: Math.max(stage.height - padding, 1) };
  return Math.min(room.width / image.width, room.height / image.height, 1);
}

/** The next zoom stop above (1) or below (-1) `current`, where 1 is fit-to-window. */
export function nextZoom(current: number, direction: 1 | -1): number {
  if (direction === 1) return ZOOM_STEPS.find((step) => step > current + 1e-6) ?? ZOOM_STEPS[ZOOM_STEPS.length - 1];
  const below = [...ZOOM_STEPS].reverse().find((step) => step < current - 1e-6);
  return below ?? ZOOM_STEPS[0];
}
