import { useSyncExternalStore } from "react";

import { type PreviewElementAnnotation } from "./browserPreview";

export interface VisualAnnotation extends PreviewElementAnnotation {
  id: string;
  sessionId: string;
  capturedAt: string;
}

const EMPTY_ANNOTATIONS: readonly VisualAnnotation[] = [];
const annotationsBySession = new Map<string, readonly VisualAnnotation[]>();
const listeners = new Set<() => void>();
let sequence = 1;

function emit(): void {
  listeners.forEach((listener) => listener());
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getVisualAnnotations(sessionId: string): readonly VisualAnnotation[] {
  return annotationsBySession.get(sessionId) ?? EMPTY_ANNOTATIONS;
}

export function useVisualAnnotations(sessionId: string): readonly VisualAnnotation[] {
  return useSyncExternalStore(
    subscribe,
    () => getVisualAnnotations(sessionId),
    () => getVisualAnnotations(sessionId),
  );
}

export function addVisualAnnotation(
  sessionId: string,
  annotation: PreviewElementAnnotation,
): VisualAnnotation {
  const created: VisualAnnotation = {
    ...annotation,
    id: `visual-${Date.now().toString(36)}-${sequence.toString(36)}`,
    sessionId,
    capturedAt: new Date().toISOString(),
  };
  sequence += 1;
  annotationsBySession.set(sessionId, [...getVisualAnnotations(sessionId), created]);
  emit();
  return created;
}

export function removeVisualAnnotation(sessionId: string, id: string): void {
  const current = getVisualAnnotations(sessionId);
  const next = current.filter((annotation) => annotation.id !== id);
  if (next.length === current.length) return;
  if (next.length === 0) annotationsBySession.delete(sessionId);
  else annotationsBySession.set(sessionId, next);
  emit();
}

export function clearVisualAnnotations(sessionId: string): void {
  if (!annotationsBySession.delete(sessionId)) return;
  emit();
}

function fenced(value: string): string {
  return value.replaceAll("`", "\\`");
}

export function visualAnnotationLabel(annotation: PreviewElementAnnotation): string {
  return annotation.accessibleName || annotation.text || annotation.selector || annotation.tagName;
}

export function formatVisualAnnotationsPrompt(
  prompt: string,
  annotations: readonly VisualAnnotation[],
): string {
  if (annotations.length === 0) return prompt.trim();
  const intro = prompt.trim() || "Please update the selected interface elements.";
  const blocks = annotations.map((annotation, index) => {
    const attributes = annotation.attributes
      .map((attribute) => `${attribute.name}=${JSON.stringify(attribute.value)}`)
      .join(" ");
    const rect = `${annotation.rect.width}×${annotation.rect.height} at ${annotation.rect.x},${annotation.rect.y}`;
    return [
      `${index + 1}. \`${fenced(annotation.selector)}\` on ${annotation.url}`,
      `   Element: <${annotation.tagName}>${annotation.role ? ` role=${annotation.role}` : ""}${
        annotation.accessibleName
          ? ` accessible name=${JSON.stringify(annotation.accessibleName)}`
          : ""
      }`,
      attributes ? `   Attributes: ${attributes}` : null,
      `   Bounds: ${rect} CSS px`,
      annotation.text ? `   Visible text: ${JSON.stringify(annotation.text)}` : null,
      annotation.title ? `   Page title: ${JSON.stringify(annotation.title)}` : null,
    ]
      .filter((line): line is string => line != null)
      .join("\n");
  });
  return `${intro}\n\nBrowser preview annotations:\n${blocks.join("\n\n")}`;
}

export const __testing = {
  reset(): void {
    annotationsBySession.clear();
    listeners.clear();
    sequence = 1;
  },
};
