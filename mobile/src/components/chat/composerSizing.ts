export const COMPOSER_LINE_HEIGHT = 22;
export const COMPOSER_MIN_HEIGHT = 44;
export const COMPOSER_MAX_LINES = 6;
export const COMPOSER_MAX_HEIGHT =
  COMPOSER_LINE_HEIGHT * COMPOSER_MAX_LINES + (COMPOSER_MIN_HEIGHT - COMPOSER_LINE_HEIGHT);

export function clampComposerHeight(contentHeight: number): number {
  return Math.min(COMPOSER_MAX_HEIGHT, Math.max(COMPOSER_MIN_HEIGHT, contentHeight));
}
