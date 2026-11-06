import type { ProjectSuggestion, RemoteInput } from "./ws";

export const projectOfferMove = (s: ProjectSuggestion): RemoteInput => ({ kind: "project_move", path: s.path });

export const projectOfferDismiss = (s: ProjectSuggestion): RemoteInput => ({ kind: "project_dismiss", path: s.path });

export function projectOfferCopy(s: ProjectSuggestion): { title: string; detail: string } {
  return {
    title: `This session is working in ${s.name}. Move it there?`,
    detail: "Moving gives it the project's instructions, Lattice index and changes view.",
  };
}
