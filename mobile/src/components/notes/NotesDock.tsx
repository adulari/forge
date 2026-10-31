// Dock wrapper for the Notes surface: follows the focused session's project and lets the person
// switch to any recent one. The phone page (app/notes.tsx) renders the same panel.
import React, { useMemo, useState } from "react";

import { useProjects, useSessions } from "../../lib/queries";
import { useSessionRow } from "../shell/activeSession";
import { NotesPanel } from "./NotesPanel";
import { noteProjectKey, noteProjects } from "./notesModel";

export function NotesDock({ sessionId }: { sessionId: string | null }) {
  const row = useSessionRow(sessionId);
  const catalog = useProjects().data;
  const sessionRows = useSessions().data;
  const [picked, setPicked] = useState<string | null>(null);
  const projects = useMemo(
    () => noteProjects(row?.cwd ?? null, catalog?.recent ?? [], sessionRows ?? []),
    [row?.cwd, catalog, sessionRows],
  );
  const own = row ? noteProjectKey(row.cwd) : null;
  // A pick sticks until the focused session changes projects, then the dock follows the session again.
  const project = picked && (own == null || picked !== own) && projects.some((p) => p.path === picked) ? picked : (own ?? projects[0]?.path ?? null);
  return <NotesPanel project={project} projects={projects} onProject={setPicked} />;
}
